//! Cycle de vie des sandboxes : backend (Docker ou local), démarrage dans
//! l'ordre du plan (ancien conteneur supprimé, ancienne clé révoquée, nouvelle
//! clé, jeton, `config.toml`, conteneur), arrêt, nettoyage au démarrage,
//! inactivité, tirage de la devbox en fond, masker.

use std::{
    collections::HashMap,
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::Instant,
};

use axum::{Json, extract::State};
use enginex_sandbox::{Backend, Handle, Mounts, Progress, Spec, docker, local::Local, spec::DockerOpts};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{
    api_keys, auth,
    config::SandboxBackend,
    error::{ApiError, ApiJson},
    mask::Masker,
    state::AppState,
    system::{set_setting, setting},
    v1::Active,
    vault,
    workspace::TopicInfo,
};
use enginex_engine::supervisor::EngineState;

/// Sandbox vivante : ce qu'elle a reçu (masquage) et son preset.
pub struct Live {
    pub injected: Vec<String>,
    pub preset: String,
}

#[derive(Default)]
pub struct Sandboxes {
    backend: OnceLock<Result<Arc<Backend>, String>>,
    host_data: OnceLock<Result<PathBuf, String>>,
    instance: OnceLock<String>,
    pub live: Mutex<HashMap<String, Live>>,
}

impl Sandboxes {
    pub fn backend(&self) -> Result<Arc<Backend>, ApiError> {
        match self.backend.get() {
            Some(Ok(b)) => Ok(b.clone()),
            _ => Err(ApiError::SandboxUnavailable),
        }
    }

    pub fn instance(&self) -> &str {
        self.instance.get().map(String::as_str).unwrap_or("")
    }
}

/// Clé d'API d'une sandbox.
pub fn key_name(session_id: &str) -> String {
    format!("sandbox:{session_id}")
}

pub fn session_dir(state: &AppState, session_id: &str) -> PathBuf {
    state.config().data_dir.join("sessions").join(session_id)
}

/// Label `enginex.preset` : id, date de modification et empreinte des
/// réglages (deux modifications dans la même seconde comptent).
pub(crate) fn preset_label(a: &Active) -> String {
    let s = serde_json::to_string(&a.settings).unwrap_or_default();
    let h = Sha256::digest(format!("{}\0{s}", a.name).as_bytes());
    let short: String = h.iter().take(4).map(|b| format!("{b:02x}")).collect();
    format!("{}:{}:{short}", a.id, a.updated_at)
}

/// Choisit le backend, découvre les chemins de l'hôte, nettoie ce qu'un
/// arrêt brutal a laissé. Appelé une fois au démarrage.
pub async fn boot(state: &AppState) -> anyhow::Result<()> {
    let sb = &state.0.sandboxes;
    let instance = match setting(state, "instance.id").await? {
        Some(i) => i,
        None => {
            let i = uuid::Uuid::new_v4().to_string();
            set_setting(state, "instance.id", Some(&i)).await?;
            i
        }
    };
    let _ = sb.instance.set(instance.clone());
    let cfg = &state.config().sandbox;
    let backend: Result<Arc<Backend>, String> = match cfg.backend {
        SandboxBackend::Local => {
            tracing::warn!(
                "sandboxes en processus local ({}) : AUCUNE isolation, réservé au développement",
                cfg.codex_bin.display()
            );
            let _ = sb.host_data.set(Ok(state.config().data_dir.clone()));
            Ok(Arc::new(Backend::Local(Local::new(
                cfg.codex_bin.clone(),
                state.config().data_dir.join("sandboxes-local"),
            ))))
        }
        SandboxBackend::Docker => match docker::client(&cfg.docker_host) {
            Err(e) => Err(format!("Docker injoignable ({}) : {e}", cfg.docker_host)),
            Ok(client) => {
                let data_dir = state.config().data_dir.clone();
                let host = match &cfg.host_data {
                    Some(p) => Ok(p.clone()),
                    None => docker::discover_host_data(&client, &data_dir)
                        .await
                        .map_err(|e| e.to_string()),
                };
                let _ = sb.host_data.set(host.clone());
                match host {
                    Ok(host_data) => Ok(Arc::new(Backend::Docker(docker::Docker::new(
                        client,
                        DockerOpts {
                            image: cfg.devbox_image.clone(),
                            network: cfg.network.clone(),
                            host_gateway: cfg.host_gateway,
                            memory: cfg.memory,
                            data_dir,
                            host_data,
                        },
                    )))),
                    Err(e) => Err(format!("Docker injoignable ou dossier /data introuvable : {e}")),
                }
            }
        },
    };
    if let Err(e) = &backend {
        tracing::error!("sessions désactivées : {e}");
    }
    let _ = sb.backend.set(backend);
    // Nettoyage : sandboxes de cette instance, clés internes, tours en cours.
    if let Ok(b) = sb.backend() {
        match b.list_owned(&instance).await {
            Ok(list) => {
                for o in list {
                    tracing::info!("sandbox {} laissée par un arrêt brutal : supprimée", o.session_id);
                    if let Err(e) = b.stop(&o.session_id).await {
                        tracing::warn!("suppression de la sandbox {} : {e}", o.session_id);
                    }
                }
            }
            Err(e) => tracing::warn!("liste des sandboxes : {e}"),
        }
        if let Err(e) = b.remove_helpers(&instance).await {
            tracing::warn!("conteneurs d'aide restants : {e}");
        }
    }
    sqlx::query("DELETE FROM api_keys WHERE internal = 1")
        .execute(state.db())
        .await?;
    sqlx::query("UPDATE chat_turns SET status = 'interrupted', completed_at = ? WHERE status = 'inProgress'")
        .bind(crate::state::now())
        .execute(state.db())
        .await?;
    sqlx::query(
        "UPDATE verifications SET status = 'error', completed_at = ?, \
         steps = json_insert(steps, '$[#]', json_object('error', 'arrêt d''EngineX')) WHERE status = 'running'",
    )
    .bind(crate::state::now())
    .execute(state.db())
    .await?;
    rebuild_masker(state).await;
    let st = state.clone();
    tokio::spawn(async move { idle_loop(st).await });
    if let (Some(every), Ok(b)) = (state.config().timings.sandbox.pull_every, sb.backend())
        && matches!(*b, Backend::Docker(_))
    {
        let st = state.clone();
        tokio::spawn(async move { pull_loop(st, every).await });
    }
    Ok(())
}

/// Reconstruit le masker : valeurs du coffre + valeurs injectées dans les
/// sandboxes vivantes.
pub async fn rebuild_masker(state: &AppState) {
    let mut values = match vault::all_values(state).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("masquage : lecture du coffre impossible : {e}");
            Vec::new()
        }
    };
    for l in state.0.sandboxes.live.lock().expect("verrou").values() {
        values.extend(l.injected.iter().cloned());
    }
    *state.0.masker.write().expect("verrou") = Arc::new(Masker::new(values));
}

fn write_private(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(data)?;
    std::fs::rename(tmp, path)
}

/// URL du serveur MCP vue de la sandbox : `api_url` sans `/v1`, plus `/mcp`.
pub fn mcp_url(api_url: &str) -> String {
    let base = api_url.trim_end_matches('/');
    format!("{}/mcp", base.strip_suffix("/v1").unwrap_or(base))
}

/// Démarre la sandbox d'une session ; rend la poignée et le jeton.
pub(crate) async fn start(
    state: &AppState,
    session_id: &str,
    topic: &TopicInfo,
    active: &Active,
    progress: &(dyn Fn(Progress) + Send + Sync),
) -> Result<(Handle, String), anyhow::Error> {
    let backend = state
        .0
        .sandboxes
        .backend()
        .map_err(|_| anyhow::anyhow!("sandboxes indisponibles"))?;
    // 1. ancien conteneur supprimé, puis sa clé révoquée.
    backend.stop(session_id).await?;
    api_keys::revoke_internal(state, &key_name(session_id)).await?;
    // 2. nouvelle clé interne.
    let key = api_keys::create_internal(state, &key_name(session_id)).await?;
    let dir = session_dir(state, session_id);
    // Copie séparée : son dossier remplace le topic comme `/workspace`.
    let copy = crate::sessions::row(state, session_id)
        .await
        .map(|r| r.copy_branch.is_some())
        .unwrap_or(false);
    let work = if copy {
        crate::snapshots::copy_dir(state, session_id)
    } else {
        topic.dir.clone()
    };
    let mounts = Mounts {
        work,
        codex_home: dir.join("codex"),
        run: dir.join("run"),
        attachments: dir.join("attachments"),
        cache: state.config().data_dir.join("cache").join(&topic.project_slug),
    };
    for d in [
        &mounts.work,
        &mounts.codex_home,
        &mounts.run,
        &mounts.attachments,
        &mounts.cache,
    ] {
        std::fs::create_dir_all(d)?;
    }
    // 3. jeton de l'app-server.
    let token = auth::random_token();
    write_private(&mounts.run.join("token"), token.as_bytes())?;
    // 4. config.toml du preset actif.
    //    Écrit sans suivre un lien laissé dans CODEX_HOME (monté en écriture).
    let api_url = &state.config().sandbox.api_url;
    let mcp_url = mcp_url(api_url);
    //    Contexte réellement chargé : si le moteur charge ce preset, on
    //    attend sa sonde (`--fit` peut avoir réduit le contexte).
    let loading =
        matches!(state.supervisor().state(), EngineState::Starting { preset_id, .. } if preset_id == active.id);
    if loading && crate::engine_api::wait_loaded(state).await {
        // La sonde suit `ready` de quelques millisecondes.
        for _ in 0..30 {
            let probed = state.0.probe.lock().expect("sonde").as_ref().map(|p| p.preset_id);
            if probed == Some(active.id) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
    let ctx = crate::engine_api::effective_ctx(state, active.id, active.settings.ctx);
    let toml = crate::codex_config::render(&active.name, ctx, api_url, Some(&mcp_url));
    write_private(&mounts.codex_home.join("config.toml"), toml.as_bytes())?;
    //    Skills actives, copiées sans suivre de lien.
    if let Err(e) = crate::skills::install_into(state, &mounts.codex_home).await {
        tracing::warn!("session {session_id} : skills non installées : {e}");
    }
    // 5. environnement, puis conteneur.
    // Ordre : identité git, secrets (un secret GIT_* l'emporte), variables d'EngineX.
    let secrets = vault::env_for(state, topic.project_id).await?;
    let injected: Vec<String> = secrets.iter().map(|(_, v)| v.clone()).collect();
    let mut env = git_env(state).await;
    env.extend(secrets);
    env.push((crate::codex_config::ENV_KEY.into(), key));
    env.extend(state.config().sandbox.extra_env.iter().cloned());
    let preset = preset_label(active);
    state.0.sandboxes.live.lock().expect("verrou").insert(
        session_id.to_string(),
        Live {
            injected,
            preset: preset.clone(),
        },
    );
    rebuild_masker(state).await;
    let spec = Spec {
        session_id: session_id.to_string(),
        owner: state.0.sandboxes.instance().to_string(),
        preset,
        mounts,
        env,
    };
    match backend.start(&spec, progress).await {
        Ok(h) => Ok((h, token)),
        Err(e) => {
            stop(state, session_id).await;
            Err(e)
        }
    }
}

/// Supprime la sandbox et révoque sa clé.
pub async fn stop(state: &AppState, session_id: &str) {
    if let Ok(b) = state.0.sandboxes.backend()
        && let Err(e) = b.stop(session_id).await
    {
        tracing::warn!("arrêt de la sandbox {session_id} : {e}");
    }
    if let Err(e) = api_keys::revoke_internal(state, &key_name(session_id)).await {
        tracing::warn!("révocation de la clé de {session_id} : {e}");
    }
    let removed = state.0.sandboxes.live.lock().expect("verrou").remove(session_id);
    if removed.is_some() {
        rebuild_masker(state).await;
    }
}

pub async fn logs(state: &AppState, session_id: &str) -> String {
    match state.0.sandboxes.backend() {
        Ok(b) => b.logs(session_id).await,
        Err(_) => String::new(),
    }
}

pub(crate) async fn setting_num(state: &AppState, key: &str, d: i64, lo: i64, hi: i64) -> i64 {
    setting(state, key)
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(d)
        .clamp(lo, hi)
}

pub async fn idle_minutes(state: &AppState) -> i64 {
    setting_num(state, "sandbox.idle_minutes", 10, 5, 1440).await
}

pub async fn max_live(state: &AppState) -> i64 {
    setting_num(state, "sandbox.max", 3, 1, 10).await
}

async fn idle_loop(state: AppState) {
    let t = state.config().timings.sandbox.clone();
    loop {
        tokio::time::sleep(t.idle_tick).await;
        let idle = match t.idle_override {
            Some(d) => d,
            None => std::time::Duration::from_secs(idle_minutes(&state).await as u64 * 60),
        };
        for hub in crate::bridge::hubs(&state) {
            // Tour de mémoire trop long : interrompu, sa fin rend l'inactivité.
            if hub.memory_turn_age().is_some_and(|d| d >= t.memory_turn_timeout) {
                tracing::info!("session {} : tour de mémoire trop long, interrompu", hub.id);
                if let Err(e) = hub.interrupt().await {
                    tracing::warn!("session {} : turn/interrupt : {e}", hub.id);
                }
                continue;
            }
            if !hub.idle_for().is_some_and(|d| d >= idle) {
                continue;
            }
            // Tour de mémoire seulement si le modèle est chargé (sinon il échouerait).
            let engine_ready = crate::v1::not_ready(&state.supervisor().state()).is_none();
            if engine_ready
                && crate::memory::should_close_with_turn(&state, &hub.id).await
                && hub.start_memory_turn(&state)
            {
                tracing::info!("sandbox {} inactive : mise à jour de la mémoire avant l'arrêt", hub.id);
            } else if hub.stop_if_idle(&state, idle, "inactivité").await {
                tracing::info!("sandbox {} inactive : arrêtée", hub.id);
            }
        }
    }
}

async fn pull_loop(state: AppState, every: std::time::Duration) {
    loop {
        pull_now(&state).await;
        tokio::time::sleep(every).await;
    }
}

/// Tirage de l'image devbox ; résultat gardé dans les réglages.
pub async fn pull_now(state: &AppState) {
    let Ok(b) = state.0.sandboxes.backend() else { return };
    let Backend::Docker(d) = &*b else { return };
    let started = Instant::now();
    let res = d.pull(&|_| {}).await;
    let t = crate::state::now().to_string();
    let _ = set_setting(state, "devbox.last_pull_at", Some(&t)).await;
    match res {
        Ok(()) => {
            tracing::info!("image devbox à jour ({} s)", started.elapsed().as_secs());
            let _ = set_setting(state, "devbox.last_pull_error", None).await;
        }
        Err(e) => {
            tracing::warn!("tirage de la devbox : {e}");
            let _ = set_setting(state, "devbox.last_pull_error", Some(&e.to_string())).await;
        }
    }
}

// ---- Routes ----

pub async fn status(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let sb = &state.0.sandboxes;
    let cfg = &state.config().sandbox;
    let (host_path, host_error) = match sb.host_data.get() {
        Some(Ok(p)) => (Some(p.display().to_string()), None),
        Some(Err(e)) => (None, Some(e.clone())),
        None => (None, None),
    };
    let (docker, image_present) = match sb.backend.get() {
        Some(Ok(b)) => match &**b {
            Backend::Docker(d) => {
                let s = d.status().await;
                (json!({"reachable": s.reachable, "error": s.error}), s.image_present)
            }
            Backend::Local(_) => (json!({"reachable": false, "error": null}), false),
        },
        Some(Err(e)) => (json!({"reachable": false, "error": e}), false),
        None => (json!({"reachable": false, "error": null}), false),
    };
    let last_at: Option<i64> = setting(&state, "devbox.last_pull_at")
        .await?
        .and_then(|v| v.parse().ok());
    let last_err = setting(&state, "devbox.last_pull_error").await?;
    Ok(Json(json!({
        "backend": cfg.backend,
        "available": sb.backend().is_ok(),
        "docker": docker,
        "host_data": {"path": host_path, "error": host_error},
        "image": {"name": cfg.devbox_image, "present": image_present,
                  "last_pull_at": last_at, "last_pull_error": last_err},
        "live": sb.live.lock().expect("verrou").len(),
    })))
}

/// Identité git de la copie séparée (défaut : EngineX).
pub async fn git_identity(state: &AppState) -> (String, String) {
    let name = setting(state, "git.name")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "EngineX".into());
    let email = setting(state, "git.email")
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "enginex@localhost".into());
    (name, email)
}

/// `GIT_AUTHOR_*` et `GIT_COMMITTER_*` du réglage Paramètres → Sessions.
pub(crate) async fn git_env(state: &AppState) -> Vec<(String, String)> {
    let (name, email) = git_identity(state).await;
    ["AUTHOR", "COMMITTER"]
        .iter()
        .flat_map(|w| {
            [
                (format!("GIT_{w}_NAME"), name.clone()),
                (format!("GIT_{w}_EMAIL"), email.clone()),
            ]
        })
        .collect()
}

pub async fn get_settings(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let (git_name, git_email) = git_identity(&state).await;
    Ok(Json(json!({
        "idle_minutes": idle_minutes(&state).await,
        "max": max_live(&state).await,
        "verify_enabled": crate::verify::enabled(&state).await,
        "verify_max_fixes": crate::verify::max_fixes(&state).await,
        "memory_auto": crate::memory::auto_enabled(&state).await,
        "git_name": git_name,
        "git_email": git_email,
    })))
}

/// Champ absent = inchangé.
#[derive(Deserialize)]
pub struct SettingsBody {
    idle_minutes: Option<i64>,
    max: Option<i64>,
    verify_enabled: Option<bool>,
    verify_max_fixes: Option<i64>,
    memory_auto: Option<bool>,
    git_name: Option<String>,
    git_email: Option<String>,
}

pub async fn put_settings(
    State(state): State<AppState>,
    ApiJson(b): ApiJson<SettingsBody>,
) -> Result<Json<Value>, ApiError> {
    let ok = b.idle_minutes.is_none_or(|v| (5..=1440).contains(&v))
        && b.max.is_none_or(|v| (1..=10).contains(&v))
        && b.verify_max_fixes.is_none_or(|v| (0..=10).contains(&v))
        && b.git_name.as_deref().is_none_or(|v| {
            let n = v.trim().chars().count();
            (1..=100).contains(&n) && !v.contains(['\n', '\r', '\0'])
        })
        && b.git_email.as_deref().is_none_or(|v| {
            let n = v.trim().chars().count();
            (1..=200).contains(&n) && v.contains('@') && !v.contains(['\n', '\r', '\0', '<', '>'])
        });
    if !ok {
        return Err(ApiError::BadRequest);
    }
    if let Some(v) = b.idle_minutes {
        set_setting(&state, "sandbox.idle_minutes", Some(&v.to_string())).await?;
    }
    if let Some(v) = b.max {
        set_setting(&state, "sandbox.max", Some(&v.to_string())).await?;
    }
    if let Some(v) = b.verify_enabled {
        set_setting(&state, "verify.enabled", Some(if v { "1" } else { "0" })).await?;
    }
    if let Some(v) = b.verify_max_fixes {
        set_setting(&state, "verify.max_fixes", Some(&v.to_string())).await?;
    }
    if let Some(v) = b.memory_auto {
        set_setting(&state, crate::memory::AUTO_KEY, Some(if v { "1" } else { "0" })).await?;
    }
    if let Some(v) = &b.git_name {
        set_setting(&state, "git.name", Some(v.trim())).await?;
    }
    if let Some(v) = &b.git_email {
        set_setting(&state, "git.email", Some(v.trim())).await?;
    }
    get_settings(State(state)).await
}
