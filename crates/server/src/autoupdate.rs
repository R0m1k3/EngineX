//! Mise à jour automatique de llama.cpp (réglage `engine.auto_update`, `on`
//! par défaut ; coupée si `ENGINEX_LLAMA_BIN` est défini).
//!
//! Contrôle du registre au démarrage (après `first_check`) puis toutes les
//! `check_every` : une version plus récente est téléchargée et testée sans
//! toucher au modèle en service, puis mise en attente (`engine.pending`). Elle
//! est mise en service au repos : aucune requête /v1 depuis `idle_for`, aucun
//! téléchargement de modèle, porte de service libre. Échec → retour à la
//! version d'avant et tag dans `engine.skipped` (sauf sonde « outils » sans
//! réponse : la version sera reproposée au contrôle suivant).

use std::sync::Mutex;

use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::time::Instant;

use crate::{
    engine_api::{self, SwitchFail, SwitchOrigin, VARIANT},
    error::{ApiError, ApiJson},
    presets,
    state::{AppState, now, now_ms},
    system::{set_setting, setting},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Pending {
    pub tag: String,
    pub build: String,
    pub since: i64,
}

/// Prochain contrôle du registre (`None` = au prochain `tick`).
#[derive(Default)]
pub struct AutoState {
    next_check: Mutex<Option<Instant>>,
}

fn build_num(b: &str) -> u64 {
    b.trim_start_matches('b').parse().unwrap_or(0)
}

pub async fn enabled(state: &AppState) -> bool {
    state.config().llama_bin.is_none()
        && setting(state, "engine.auto_update").await.ok().flatten().as_deref() != Some("off")
}

pub async fn pending(state: &AppState) -> Option<Pending> {
    let raw = setting(state, "engine.pending").await.ok().flatten()?;
    serde_json::from_str(&raw).ok()
}

async fn set_pending(state: &AppState, p: Option<&Pending>) {
    let v = p.map(|p| serde_json::to_string(p).expect("json"));
    if let Err(e) = set_setting(state, "engine.pending", v.as_deref()).await {
        tracing::warn!("engine.pending : {e}");
    }
}

pub async fn skipped(state: &AppState) -> Vec<String> {
    setting(state, "engine.skipped")
        .await
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default()
}

async fn set_skipped(state: &AppState, list: &[String]) {
    let v = (!list.is_empty()).then(|| serde_json::to_string(list).expect("json"));
    if let Err(e) = set_setting(state, "engine.skipped", v.as_deref()).await {
        tracing::warn!("engine.skipped : {e}");
    }
}

/// Écarte `tag` : plus jamais retenté automatiquement.
pub async fn skip(state: &AppState, tag: &str) {
    let mut l = skipped(state).await;
    if !l.iter().any(|t| t == tag) {
        l.push(tag.to_string());
        set_skipped(state, &l).await;
    }
}

/// Une installation à la main remet la version en jeu.
pub async fn unskip(state: &AppState, tag: &str) {
    let l = skipped(state).await;
    if l.iter().any(|t| t == tag) {
        let rest: Vec<String> = l.into_iter().filter(|t| t != tag).collect();
        set_skipped(state, &rest).await;
    }
}

async fn last(state: &AppState, tag: &str, ok: bool, message: &str) {
    let v = json!({"at": now(), "tag": tag, "ok": ok, "message": message}).to_string();
    if let Err(e) = set_setting(state, "engine.auto_last", Some(&v)).await {
        tracing::warn!("engine.auto_last : {e}");
    }
}

pub async fn auto_last(state: &AppState) -> Option<Value> {
    setting(state, "engine.auto_last")
        .await
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_str(&v).ok())
}

async fn current_build(state: &AppState) -> u64 {
    let Ok(tag) = engine_api::active_tag(state).await else {
        return 0;
    };
    let b: Option<String> = sqlx::query_scalar("SELECT build FROM engine_versions WHERE tag = ?")
        .bind(&tag)
        .fetch_optional(state.db())
        .await
        .ok()
        .flatten();
    b.as_deref().map(build_num).unwrap_or(0)
}

/// Boucle de fond (lancée par `start_background` si `timings.auto_loop`).
pub async fn run(state: AppState) {
    let t = state.config().timings.auto.clone();
    tokio::time::sleep(t.first_check).await;
    loop {
        tick(&state).await;
        tokio::time::sleep(t.tick).await;
    }
}

/// Une itération : contrôle du registre s'il est dû, puis mise en service de
/// la version en attente si EngineX est au repos.
pub async fn tick(state: &AppState) {
    if !enabled(state).await {
        return;
    }
    let due = {
        let n = state.0.auto.next_check.lock().expect("auto");
        n.is_none_or(|t| t <= Instant::now())
    };
    if due {
        check(state).await;
    }
    activate_pending(state).await;
}

async fn check(state: &AppState) {
    let every = state.config().timings.auto.check_every;
    *state.0.auto.next_check.lock().expect("auto") = Some(Instant::now() + every);
    let rel = match state.0.registry.latest(VARIANT).await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("mise à jour automatique : registre injoignable ({e})");
            return;
        }
    };
    *state.0.latest.lock().expect("latest") = Some(rel.clone());
    let old = pending(state).await;
    if build_num(&rel.build) <= current_build(state).await
        || skipped(state).await.contains(&rel.tag)
        || old.as_ref().is_some_and(|p| p.tag == rel.tag)
    {
        return;
    }
    if engine_api::begin_job(state, "auto_update", &rel.tag).is_err() {
        // Une autre tâche tourne : nouvel essai au prochain tick.
        *state.0.auto.next_check.lock().expect("auto") = None;
        return;
    }
    tracing::info!("mise à jour automatique : {} trouvée, téléchargement", rel.tag);
    match engine_api::download_version(state, &rel).await {
        Ok(()) => {
            let p = Pending {
                tag: rel.tag.clone(),
                build: rel.build.clone(),
                since: now(),
            };
            set_pending(state, Some(&p)).await;
            if let Some(o) = old.filter(|o| o.tag != rel.tag) {
                // Remplacée par plus récente : l'ancienne en attente part.
                remove_if_unused(state, &o.tag).await;
            }
            engine_api::end_job(state, Ok(format!("{} prête, en attente de repos", rel.tag)));
        }
        Err(e) => {
            skip(state, &rel.tag).await;
            last(state, &rel.tag, false, &e).await;
            engine_api::end_job(state, Err(e));
        }
    }
}

async fn remove_if_unused(state: &AppState, tag: &str) {
    let active = engine_api::active_tag(state).await.ok();
    let prev = setting(state, "engine.previous").await.ok().flatten();
    if active.as_deref() != Some(tag) && prev.as_deref() != Some(tag) {
        let _ = engine_api::remove_version(state, tag).await;
    }
}

async fn activate_pending(state: &AppState) {
    let Some(p) = pending(state).await else { return };
    if !engine_api::bin_of(state, &p.tag).is_file() {
        set_pending(state, None).await;
        return;
    }
    let has_preset = presets::active_id(state).await.ok().flatten().is_some();
    if has_preset {
        let idle = state.config().timings.auto.idle_for.as_millis() as i64;
        if now_ms() - state.last_done_ms() < idle || state.0.downloads.busy() {
            return;
        }
    }
    let previous_job = state.0.engine_job.lock().expect("job").clone();
    if engine_api::begin_job(state, "auto_update", &p.tag).is_err() {
        return;
    }
    let Ok(gate) = state.serve_gate().clone().try_write_owned() else {
        // Une requête est en cours : la tâche se referme sans résultat.
        *state.0.engine_job.lock().expect("job") = previous_job;
        return;
    };
    tracing::info!("mise à jour automatique : mise en service de {}", p.tag);
    let res = engine_api::switch_to(state, &p.tag, SwitchOrigin::Auto, gate).await;
    set_pending(state, None).await;
    match res {
        Ok(msg) => {
            last(state, &p.tag, true, &msg).await;
            engine_api::end_job(state, Ok(msg));
        }
        Err(e) => {
            if e.kind == SwitchFail::Failed {
                skip(state, &p.tag).await;
            }
            last(state, &p.tag, false, &e.message).await;
            engine_api::end_job(state, Err(e.message));
        }
    }
}

#[derive(Deserialize)]
pub struct AutoBody {
    on: bool,
}

/// `PUT /api/engine/auto-update {on}`.
pub async fn set_auto(State(state): State<AppState>, ApiJson(b): ApiJson<AutoBody>) -> Result<Json<Value>, ApiError> {
    if state.config().llama_bin.is_some() {
        return Err(ApiError::EnvOverride);
    }
    set_setting(&state, "engine.auto_update", Some(if b.on { "on" } else { "off" })).await?;
    if b.on {
        // Contrôle au prochain tick.
        *state.0.auto.next_check.lock().expect("auto") = None;
    }
    Ok(Json(json!({ "on": b.on })))
}
