//! Moteur : lancement du preset actif, état, logs, versions de llama.cpp
//! (vérifier, installer, utiliser, revenir à la précédente).

use std::{
    path::{Path, PathBuf},
    sync::{LazyLock, atomic::Ordering},
    time::Duration,
};

use axum::{
    Json,
    extract::{Path as UrlPath, Query, State},
};
use enginex_engine::{
    args::{PresetSettings, build_argv},
    oci::{Release, UpdateError},
    probe,
    resolve::{IMAGE_TAG, Origin, installed_bin},
    supervisor::{EngineState, LaunchSpec},
    update::{self, TestReport},
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::OwnedRwLockWriteGuard;

use crate::{
    error::{ApiError, ApiJson},
    models::{self, model_path},
    presets,
    state::{AppState, EngineProbe, READY_TIMEOUT, now},
    system::{current_bin, reset_cache, set_setting, setting},
};

/// Variante suivie des images llama.cpp.
pub const VARIANT: &str = "server-cuda";

static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(image|server-cuda-b[0-9]+)$").expect("motif"));

/// Installation ou bascule de moteur (la dernière reste visible une fois finie).
#[derive(Clone, Debug, Serialize)]
pub struct JobState {
    pub kind: &'static str,
    pub tag: String,
    /// `kind` : `install`, `use`, `rollback`, `auto_update`, `activate`,
    /// `stop`, `delete`. `step` : `download`, `test`, `wait_requests` (attente
    /// de la fin des requêtes /v1), `loading`, `done`, `failed`.
    pub step: &'static str,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub finished: bool,
    pub error: Option<String>,
    pub message: Option<String>,
}

pub(crate) fn set_job(state: &AppState, f: impl FnOnce(&mut JobState)) {
    if let Some(j) = state.0.engine_job.lock().expect("job").as_mut() {
        f(j);
    }
}

/// Réserve le créneau de tâche (409 si une tâche tourne déjà). Toujours
/// avant la porte de service, jamais l'inverse.
pub(crate) fn begin_job(state: &AppState, kind: &'static str, tag: &str) -> Result<(), ApiError> {
    let mut g = state.0.engine_job.lock().expect("job");
    if g.as_ref().is_some_and(|j| !j.finished) {
        return Err(ApiError::JobRunning);
    }
    *g = Some(JobState {
        kind,
        tag: tag.to_string(),
        step: "test",
        done_bytes: 0,
        total_bytes: 0,
        finished: false,
        error: None,
        message: None,
    });
    Ok(())
}

pub(crate) fn end_job(state: &AppState, res: Result<String, String>) {
    set_job(state, |j| {
        j.finished = true;
        match res {
            Ok(m) => {
                j.step = "done";
                j.message = Some(m);
            }
            Err(e) => {
                j.step = "failed";
                j.error = Some(e);
            }
        }
    });
}

/// Prend la porte de service en écriture (étape `wait_requests` tant que des
/// requêtes /v1 sont en cours).
pub(crate) async fn take_gate(state: &AppState) -> OwnedRwLockWriteGuard<()> {
    set_job(state, |j| j.step = "wait_requests");
    let g = state.serve_gate().clone().write_owned().await;
    set_job(state, |j| j.step = "loading");
    g
}

// ---- Lancement ----

/// Mesure la VRAM prise par d'autres programmes (moteur d'EngineX arrêté).
pub async fn measure_vram_others(state: &AppState) {
    if !matches!(
        state.supervisor().state(),
        EngineState::Stopped | EngineState::Failed { .. }
    ) {
        return;
    }
    if let Ok(g) = probe::read_gpus(&state.config().nvidia_smi).await {
        let used = g.iter().map(|g| g.used_mib.max(0) as u64).collect();
        *state.0.vram_others.lock().expect("vram") = Some((now(), used));
    }
}

/// Lance le preset `id` avec le moteur courant (arrête l'éventuel précédent).
pub async fn launch(state: &AppState, id: i64) -> Result<(), ApiError> {
    let p = presets::preset(state, id).await?;
    let s: PresetSettings = serde_json::from_str(&p.settings)?;
    let m = models::model(state, p.model_id).await?;
    let dir = &state.config().models_dir;
    let model = model_path(dir, &m.repo, &m.file)?;
    let mmproj = match (&m.mmproj, s.mmproj) {
        (Some(f), true) => Some(model_path(dir, &m.repo, f)?),
        _ => None,
    };
    let (bin, _) = current_bin(state).await?;
    let has_fit = probe::help(&bin).await.is_ok_and(|h| probe::has_fit(&h));
    let argv = build_argv(&s, &model, mmproj.as_deref(), state.config().llama_port, has_fit);
    *state.0.probe.lock().expect("sonde") = None;
    state
        .supervisor()
        .start(LaunchSpec {
            bin,
            argv,
            model_bytes: m.size_bytes.max(1) as u64,
            preset_id: id,
        })
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))
}

/// Attend la fin du chargement : `true` si prêt.
pub(crate) async fn wait_loaded(state: &AppState) -> bool {
    let mut rx = state.supervisor().subscribe();
    let r = tokio::time::timeout(
        READY_TIMEOUT + Duration::from_secs(30),
        rx.wait_for(|s| {
            matches!(
                s,
                EngineState::Ready { .. } | EngineState::Failed { .. } | EngineState::Stopped
            )
        }),
    )
    .await;
    matches!(r, Ok(Ok(s)) if matches!(*s, EngineState::Ready { .. }))
}

/// Suit l'état du moteur : sondes à `ready`, VRAM des autres à l'arrêt.
fn watch_engine(state: &AppState) {
    let s = state.clone();
    let mut rx = state.supervisor().subscribe();
    tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            let st = rx.borrow_and_update().clone();
            match st {
                EngineState::Ready { preset_id, .. } => {
                    let port = s.config().llama_port;
                    let props = probe::props(&s.0.http, port).await.ok();
                    let tools = probe::tools_probe(&s.0.http, port).await.ok();
                    let n_ctx = props
                        .as_ref()
                        .and_then(|p| p.pointer("/default_generation_settings/n_ctx")?.as_u64());
                    let build = props
                        .as_ref()
                        .and_then(|p| p.get("build_info")?.as_str().map(str::to_string));
                    *s.0.probe.lock().expect("sonde") = Some(EngineProbe {
                        preset_id,
                        n_ctx,
                        tools_ok: tools,
                        build,
                    });
                }
                EngineState::Stopped => measure_vram_others(&s).await,
                _ => {}
            }
        }
    });
}

fn mtime(p: &Path) -> i64 {
    std::fs::metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn test_json(r: &Result<TestReport, UpdateError>) -> String {
    let v = match r {
        Ok(t) => json!({"ok": true, "code": null, "message": null, "gpus": t.gpus, "has_fit": t.has_fit, "at": now()}),
        Err(e) => json!({"ok": false, "code": e.code(), "message": e.to_string(), "at": now()}),
    };
    v.to_string()
}

async fn record_version(state: &AppState, tag: &str, build: &str, bin: &Path, test: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO engine_versions (tag, build, path, bin_mtime, installed_at, last_test) VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT(tag) DO UPDATE SET build = excluded.build, path = excluded.path, \
         bin_mtime = excluded.bin_mtime, last_test = excluded.last_test",
    )
    .bind(tag)
    .bind(build)
    .bind(bin.display().to_string())
    .bind(mtime(bin))
    .bind(now())
    .bind(test)
    .execute(&state.0.db)
    .await
    .map(|_| ())
}

/// Au lancement d'EngineX : build de l'image relu, moteur actif testé s'il a
/// changé (sinon secours), puis relance du preset actif.
pub async fn boot(state: &AppState) -> anyhow::Result<()> {
    watch_engine(state);
    let cfg = state.config().clone();
    let image = update::describe(&cfg.image_bin).await;
    if let Ok(r) = &image {
        record_version(state, IMAGE_TAG, &r.build, &cfg.image_bin, &test_json(&image)).await?;
    }
    let (bin, origin) = current_bin(state).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    match origin {
        Origin::Active(tag) => {
            let row: Option<(String, i64, Option<String>)> =
                sqlx::query_as("SELECT build, bin_mtime, last_test FROM engine_versions WHERE tag = ?")
                    .bind(&tag)
                    .fetch_optional(&state.0.db)
                    .await?;
            let tested_ok = row.as_ref().is_some_and(|(_, m, t)| {
                *m == mtime(&bin)
                    && t.as_deref()
                        .and_then(|t| serde_json::from_str::<Value>(t).ok())
                        .is_some_and(|v| v["ok"] == true)
            });
            if !tested_ok {
                let gpus = probe::read_gpus(&cfg.nvidia_smi).await.map(|g| g.len()).unwrap_or(0);
                let build = row.as_ref().map(|(b, _, _)| b.clone());
                let res = update::test_engine(&bin, build.as_deref(), gpus).await;
                let b = res
                    .as_ref()
                    .map(|r| r.build.clone())
                    .unwrap_or_else(|_| build.clone().unwrap_or_default());
                record_version(state, &tag, &b, &bin, &test_json(&res)).await?;
                if let Err(e) = res {
                    tracing::warn!("moteur {tag} refusé au lancement ({e}) : moteur de secours (image)");
                    state.0.fallback.store(true, Ordering::SeqCst);
                }
            }
        }
        Origin::Image => {
            // engine.active absent sur le disque : secours si un tag était choisi.
            if setting(state, "engine.active").await?.is_some_and(|t| t != IMAGE_TAG) {
                state.0.fallback.store(true, Ordering::SeqCst);
            }
        }
        Origin::Env => {}
    }
    if let Some(id) = presets::active_id(state).await.map_err(|e| anyhow::anyhow!("{e}"))? {
        // Chargement en fond, porte prise : le serveur démarre tout de suite et
        // les requêtes /v1 attendent la fin du chargement.
        let gate = state.serve_gate().clone().write_owned().await;
        let s = state.clone();
        tokio::spawn(async move {
            let _gate = gate;
            match launch(&s, id).await {
                Ok(()) => {
                    wait_loaded(&s).await;
                }
                Err(e) => tracing::warn!("relance du preset {id} : {e}"),
            }
        });
    }
    Ok(())
}

// ---- Routes : état ----

pub async fn engine(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let st = state.supervisor().state();
    let (bin, origin) = current_bin(&state).await?;
    let fallback = state.0.fallback.load(Ordering::SeqCst) && state.config().llama_bin.is_none();
    let origin = match origin {
        Origin::Env => "env",
        Origin::Active(_) => "active",
        Origin::Image if fallback => "secours",
        Origin::Image => "image",
    };
    let gpus = probe::read_gpus(&state.config().nvidia_smi).await.unwrap_or_default();
    let probe = state
        .0
        .probe
        .lock()
        .expect("sonde")
        .clone()
        .filter(|p| Some(p.preset_id) == st.preset_id());
    let tag = active_tag(&state).await?;
    let build: Option<String> = sqlx::query_scalar("SELECT build FROM engine_versions WHERE tag = ?")
        .bind(if origin == "active" { tag.as_str() } else { IMAGE_TAG })
        .fetch_optional(&state.0.db)
        .await?;
    let has_fit = probe::help(&bin).await.is_ok_and(|h| probe::has_fit(&h));
    let others = state.0.vram_others.lock().expect("vram").clone();
    Ok(Json(json!({
        "state": st,
        "preset_id": presets::active_id(&state).await?,
        "gpus": gpus,
        "tools_ok": probe.as_ref().and_then(|p| p.tools_ok),
        "n_ctx": probe.as_ref().and_then(|p| p.n_ctx),
        "build": build,
        "origin": origin,
        "has_fit": has_fit,
        "vram_others": others.as_ref().map(|(_, v)| v.clone()),
        "measured_at": others.map(|(t, _)| t),
        "auto_update": crate::autoupdate::enabled(&state).await,
        "pending": crate::autoupdate::pending(&state).await,
        "auto_last": crate::autoupdate::auto_last(&state).await,
        "skipped": crate::autoupdate::skipped(&state).await,
    })))
}

/// Arrêt du moteur : tâche en fond qui attend la fin des requêtes en cours.
pub async fn stop(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    begin_job(&state, "stop", "")?;
    let s = state.clone();
    tokio::spawn(async move {
        let _gate = take_gate(&s).await;
        s.supervisor().stop(false).await;
        let r = set_setting(&s, "preset.active", None)
            .await
            .map(|_| "moteur arrêté".to_string())
            .map_err(|e| e.to_string());
        end_job(&s, r);
    });
    Ok(Json(json!({ "started": true })))
}

#[derive(Deserialize)]
pub struct LogsQuery {
    tail: Option<usize>,
}

pub async fn logs(State(state): State<AppState>, Query(q): Query<LogsQuery>) -> Json<Value> {
    Json(json!({ "lines": state.supervisor().logs(q.tail.unwrap_or(200).min(2000)) }))
}

// ---- Routes : versions ----

pub(crate) async fn active_tag(state: &AppState) -> Result<String, ApiError> {
    Ok(setting(state, "engine.active")
        .await?
        .unwrap_or_else(|| IMAGE_TAG.to_string()))
}

pub(crate) fn bin_of(state: &AppState, tag: &str) -> PathBuf {
    if tag == IMAGE_TAG {
        state.config().image_bin.clone()
    } else {
        installed_bin(&state.config().engines_dir(), tag)
    }
}

pub async fn versions(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let rows: Vec<(String, String, i64, Option<String>)> =
        sqlx::query_as("SELECT tag, build, installed_at, last_test FROM engine_versions ORDER BY installed_at DESC")
            .fetch_all(&state.0.db)
            .await?;
    let installed: Vec<Value> = rows
        .into_iter()
        .map(|(tag, build, at, t)| {
            json!({"tag": tag, "build": build, "installed_at": at,
                   "last_test": t.and_then(|t| serde_json::from_str::<Value>(&t).ok())})
        })
        .collect();
    let latest = state.0.latest.lock().expect("latest").clone();
    let job = state.0.engine_job.lock().expect("job").clone();
    Ok(Json(json!({
        "active": active_tag(&state).await?,
        "previous": setting(&state, "engine.previous").await?,
        "installed": installed,
        "latest": latest.map(|r| json!({"tag": r.tag, "build": r.build})),
        "job": job,
        "env_override": state.config().llama_bin.is_some(),
    })))
}

fn upstream(e: UpdateError) -> ApiError {
    ApiError::Upstream(e.to_string())
}

pub async fn check(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let rel = state.0.registry.latest(VARIANT).await.map_err(upstream)?;
    let out = json!({"tag": rel.tag, "build": rel.build});
    *state.0.latest.lock().expect("latest") = Some(rel);
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct InstallBody {
    tag: Option<String>,
}

pub async fn install(State(state): State<AppState>, ApiJson(b): ApiJson<InstallBody>) -> Result<Json<Value>, ApiError> {
    if state.config().llama_bin.is_some() {
        return Err(ApiError::EnvOverride);
    }
    let cached = state.0.latest.lock().expect("latest").clone();
    let rel: Release = match cached {
        Some(r) => r,
        None => state.0.registry.latest(VARIANT).await.map_err(upstream)?,
    };
    if b.tag.as_ref().is_some_and(|t| *t != rel.tag) {
        // Seule la dernière version publiée s'installe.
        return Err(ApiError::BadRequest);
    }
    begin_job(&state, "install", &rel.tag)?;
    let s = state.clone();
    tokio::spawn(async move {
        // Installer à la main une version écartée la remet en jeu.
        crate::autoupdate::unskip(&s, &rel.tag).await;
        let r = match download_version(&s, &rel).await {
            Ok(()) => {
                let gate = take_gate(&s).await;
                switch_to(&s, &rel.tag, SwitchOrigin::Manual, gate)
                    .await
                    .map_err(|e| e.message)
            }
            Err(e) => Err(e),
        };
        end_job(&s, r);
    });
    Ok(Json(json!({ "started": true })))
}

/// Téléchargement, enregistrement et test du binaire de `rel`, sans la porte
/// de service (les requêtes /v1 passent pendant ce temps).
pub(crate) async fn download_version(state: &AppState, rel: &Release) -> Result<(), String> {
    let dir = state.config().engines_dir();
    tokio::fs::create_dir_all(&dir).await.map_err(|e| e.to_string())?;
    set_job(state, |j| j.step = "download");
    let s = state.clone();
    let bin = state
        .0
        .registry
        .install(rel, &dir, &move |done, total| {
            set_job(&s, |j| {
                j.done_bytes = done;
                j.total_bytes = total;
            })
        })
        .await;
    let bin = match bin {
        Ok(b) => b,
        Err(e) => {
            let _ = sqlx::query("UPDATE engine_versions SET last_test = ? WHERE tag = ?")
                .bind(test_json(&Err(e.clone())))
                .bind(&rel.tag)
                .execute(&state.0.db)
                .await;
            return Err(format!("{} : {e}", e.code()));
        }
    };
    set_job(state, |j| j.step = "test");
    let gpus = probe::read_gpus(&state.config().nvidia_smi)
        .await
        .map(|g| g.len())
        .unwrap_or(0);
    let res = update::test_engine(&bin, Some(&rel.build), gpus).await;
    record_version(state, &rel.tag, &rel.build, &bin, &test_json(&res))
        .await
        .map_err(|e| e.to_string())?;
    res.map(|_| ()).map_err(|e| format!("{} : {e}", e.code()))
}

pub async fn use_version(
    State(state): State<AppState>,
    UrlPath(tag): UrlPath<String>,
) -> Result<Json<Value>, ApiError> {
    if !TAG.is_match(&tag) {
        return Err(ApiError::BadRequest);
    }
    if state.config().llama_bin.is_some() {
        return Err(ApiError::EnvOverride);
    }
    if !bin_of(&state, &tag).is_file() {
        return Err(ApiError::NotFound);
    }
    begin_job(&state, "use", &tag)?;
    spawn_switch(state, tag);
    Ok(Json(json!({ "started": true })))
}

fn spawn_switch(state: AppState, tag: String) {
    tokio::spawn(async move {
        let gate = take_gate(&state).await;
        let r = switch_to(&state, &tag, SwitchOrigin::Manual, gate)
            .await
            .map_err(|e| e.message);
        end_job(&state, r);
    });
}

pub async fn rollback(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    if state.config().llama_bin.is_some() {
        return Err(ApiError::EnvOverride);
    }
    let prev = setting(&state, "engine.previous").await?.ok_or(ApiError::NotFound)?;
    if !bin_of(&state, &prev).is_file() {
        return Err(ApiError::NotFound);
    }
    begin_job(&state, "rollback", &prev)?;
    spawn_switch(state, prev);
    Ok(Json(json!({ "started": true })))
}

/// Qui a demandé la bascule.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SwitchOrigin {
    Manual,
    Auto,
}

/// Échec d'une bascule ; le moteur d'avant est de nouveau en service.
#[derive(Debug)]
pub struct SwitchError {
    pub message: String,
    pub kind: SwitchFail,
}

#[derive(Debug, PartialEq)]
pub enum SwitchFail {
    /// Test du binaire ou chargement raté.
    Failed,
    /// La sonde « outils » passait avant, elle échoue avec la nouvelle version
    /// (tag ajouté à `engine.skipped`).
    ToolsBroken,
    /// La sonde passait avant, elle n'a pas répondu (erreur passagère possible).
    ToolsUnknown,
}

fn fail(kind: SwitchFail, message: String) -> SwitchError {
    SwitchError { message, kind }
}

fn tools_ok(state: &AppState) -> Option<bool> {
    state.0.probe.lock().expect("sonde").as_ref().and_then(|p| p.tools_ok)
}

/// Attend que `watch_engine` ait rempli la sonde du nouveau chargement.
async fn wait_probe(state: &AppState) -> Option<bool> {
    let deadline = tokio::time::Instant::now() + state.config().timings.probe_wait;
    while tokio::time::Instant::now() < deadline {
        if let Some(p) = state.0.probe.lock().expect("sonde").as_ref() {
            return p.tools_ok;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    None
}

/// Teste `tag`, bascule (`previous` ← actif), relance le preset actif ; si le
/// chargement échoue ou si la sonde « outils » régresse, revient au moteur
/// d'avant (chargement attendu). Reçoit la porte en écriture déjà prise et la
/// garde jusqu'au bout.
pub(crate) async fn switch_to(
    state: &AppState,
    tag: &str,
    origin: SwitchOrigin,
    gate: OwnedRwLockWriteGuard<()>,
) -> Result<String, SwitchError> {
    let _gate = gate;
    let failed = |m: String| fail(SwitchFail::Failed, m);
    let bin = bin_of(state, tag);
    let (cur, _) = current_bin(state).await.map_err(|e| failed(e.to_string()))?;
    let gpus = update::describe(&cur).await.map(|r| r.gpus).unwrap_or(0);
    set_job(state, |j| j.step = "test");
    let res = if tag == IMAGE_TAG {
        update::describe(&bin).await
    } else {
        let build: Option<String> = sqlx::query_scalar("SELECT build FROM engine_versions WHERE tag = ?")
            .bind(tag)
            .fetch_optional(&state.0.db)
            .await
            .map_err(|e| failed(e.to_string()))?;
        update::test_engine(&bin, build.as_deref(), gpus).await
    };
    let build = res.as_ref().map(|r| r.build.clone()).unwrap_or_default();
    let _ = sqlx::query("UPDATE engine_versions SET last_test = ?, bin_mtime = ? WHERE tag = ?")
        .bind(test_json(&res))
        .bind(mtime(&bin))
        .bind(tag)
        .execute(&state.0.db)
        .await;
    if let Err(e) = res {
        return Err(failed(format!("{} : {e}", e.code())));
    }
    let old_active = active_tag(state).await.map_err(|e| failed(e.to_string()))?;
    let old_prev = setting(state, "engine.previous")
        .await
        .map_err(|e| failed(e.to_string()))?;
    let old_tools = tools_ok(state);
    let was_fallback = state.0.fallback.swap(false, Ordering::SeqCst);
    set_settings(state, tag, Some(&old_active)).await.map_err(failed)?;
    reset_cache(state).await;
    if let Some(id) = presets::active_id(state).await.map_err(|e| failed(e.to_string()))? {
        set_job(state, |j| j.step = "loading");
        let ok = launch(state, id).await.is_ok() && wait_loaded(state).await;
        let verdict = if !ok {
            let cause = match state.supervisor().state() {
                EngineState::Failed { cause, .. } => cause,
                _ => "chargement échoué".into(),
            };
            Some(fail(SwitchFail::Failed, format!("load_failed : {cause}")))
        } else {
            match (old_tools, wait_probe(state).await) {
                (Some(true), Some(false)) => Some(fail(
                    SwitchFail::ToolsBroken,
                    "tools_broken : la nouvelle version ne rend plus les appels d'outils".into(),
                )),
                (Some(true), None) => Some(fail(
                    SwitchFail::ToolsUnknown,
                    "tools_unknown : la sonde « outils » n'a pas répondu".into(),
                )),
                _ => None,
            }
        };
        if let Some(mut err) = verdict {
            set_settings(state, &old_active, old_prev.as_deref())
                .await
                .map_err(failed)?;
            state.0.fallback.store(was_fallback, Ordering::SeqCst);
            reset_cache(state).await;
            if launch(state, id).await.is_ok() {
                wait_loaded(state).await;
            }
            if err.kind == SwitchFail::ToolsBroken {
                crate::autoupdate::skip(state, tag).await;
            }
            tracing::warn!("bascule vers {tag} ({origin:?}) annulée : {}", err.message);
            err.message = format!("{} ; retour à {old_active}", err.message);
            return Err(err);
        }
    }
    cleanup(state).await;
    tracing::info!("moteur {tag} ({build}) actif ({origin:?})");
    Ok(format!("moteur {tag} ({build}) actif"))
}

async fn set_settings(state: &AppState, active: &str, previous: Option<&str>) -> Result<(), String> {
    set_setting(state, "engine.active", Some(active))
        .await
        .map_err(|e| e.to_string())?;
    set_setting(state, "engine.previous", previous)
        .await
        .map_err(|e| e.to_string())
}

/// Garde l'actif, le précédent, `image` et la version en attente ; supprime
/// les autres.
pub(crate) async fn cleanup(state: &AppState) {
    let keep = [
        active_tag(state).await.ok(),
        setting(state, "engine.previous").await.ok().flatten(),
        Some(IMAGE_TAG.to_string()),
        crate::autoupdate::pending(state).await.map(|p| p.tag),
    ];
    let tags: Vec<String> = sqlx::query_scalar("SELECT tag FROM engine_versions")
        .fetch_all(&state.0.db)
        .await
        .unwrap_or_default();
    for t in tags {
        if !keep.iter().flatten().any(|k| *k == t) {
            let _ = remove_version(state, &t).await;
        }
    }
}

pub(crate) async fn remove_version(state: &AppState, tag: &str) -> Result<(), ApiError> {
    if tag != IMAGE_TAG && TAG.is_match(tag) {
        let dir = state.config().engines_dir().join(tag);
        if tokio::fs::metadata(&dir).await.is_ok() {
            tokio::fs::remove_dir_all(&dir).await?;
        }
    }
    sqlx::query("DELETE FROM engine_versions WHERE tag = ?")
        .bind(tag)
        .execute(&state.0.db)
        .await?;
    Ok(())
}

pub async fn delete_version(
    State(state): State<AppState>,
    UrlPath(tag): UrlPath<String>,
) -> Result<Json<Value>, ApiError> {
    if !TAG.is_match(&tag) {
        return Err(ApiError::BadRequest);
    }
    if tag == IMAGE_TAG || tag == active_tag(&state).await? {
        return Err(ApiError::EngineInUse);
    }
    if state
        .0
        .engine_job
        .lock()
        .expect("job")
        .as_ref()
        .is_some_and(|j| !j.finished)
    {
        return Err(ApiError::JobRunning);
    }
    if setting(&state, "engine.previous").await?.as_deref() == Some(tag.as_str()) {
        set_setting(&state, "engine.previous", None).await?;
    }
    remove_version(&state, &tag).await?;
    Ok(Json(json!({ "ok": true })))
}
