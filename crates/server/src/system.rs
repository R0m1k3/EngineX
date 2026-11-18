//! Page Système : version et périphériques vus par llama-server, GPU vus
//! par nvidia-smi (sondes dans `enginex_engine::probe`).

use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use axum::{
    Json,
    extract::{Query, State},
};
use enginex_engine::{
    probe::{Gpu, LlamaInfo, read_gpus, read_llama},
    resolve::{Origin, resolve_bin},
};
use serde::Deserialize;

use crate::{error::ApiError, state::AppState};

/// `--list-devices` initialise CUDA : lecture gardée 5 minutes.
const LLAMA_TTL: Duration = Duration::from_secs(300);
const GPU_TTL: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct SysCache {
    llama: Option<(Instant, LlamaInfo)>,
    gpus: Option<(Instant, Result<Vec<Gpu>, String>)>,
}

/// Valeur d'un réglage (`settings`), `None` si absent.
pub async fn setting(state: &AppState, key: &str) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(&state.0.db)
        .await
}

/// Écrit (`Some`) ou efface (`None`) un réglage.
pub async fn set_setting(state: &AppState, key: &str, value: Option<&str>) -> Result<(), sqlx::Error> {
    match value {
        Some(v) => sqlx::query(
            "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(key)
        .bind(v)
        .execute(&state.0.db)
        .await
        .map(|_| ()),
        None => sqlx::query("DELETE FROM settings WHERE key = ?")
            .bind(key)
            .execute(&state.0.db)
            .await
            .map(|_| ()),
    }
}

/// Vide le cache de la page Système (à chaque bascule de moteur).
pub async fn reset_cache(state: &AppState) {
    *state.0.sys_cache.lock().await = SysCache::default();
}

/// Binaire llama-server à utiliser (voir `engine::resolve`).
pub async fn current_bin(state: &AppState) -> Result<(PathBuf, Origin), ApiError> {
    let cfg = state.config();
    let active = setting(state, "engine.active").await.map_err(anyhow::Error::from)?;
    let (bin, origin) = resolve_bin(
        cfg.llama_bin.as_deref(),
        active.as_deref(),
        &cfg.engines_dir(),
        &cfg.image_bin,
    );
    // Moteur de secours : l'actif a échoué au lancement d'EngineX.
    if matches!(origin, Origin::Active(_)) && state.0.fallback.load(std::sync::atomic::Ordering::SeqCst) {
        return Ok((cfg.image_bin.clone(), Origin::Image));
    }
    Ok((bin, origin))
}

#[derive(Deserialize)]
pub struct SystemQuery {
    refresh: Option<String>,
}

pub async fn system(
    State(state): State<AppState>,
    Query(q): Query<SystemQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let cfg = state.config();
    let (bin, origin) = current_bin(&state).await?;
    let mut cache = state.0.sys_cache.lock().await;
    if q.refresh.as_deref().is_some_and(|v| v == "1" || v == "true") {
        *cache = SysCache::default();
    }
    let now = Instant::now();
    if !cache
        .llama
        .as_ref()
        .is_some_and(|(t, l)| now.duration_since(*t) < LLAMA_TTL && l.bin == bin.display().to_string())
    {
        cache.llama = Some((now, read_llama(&bin).await));
    }
    if !cache
        .gpus
        .as_ref()
        .is_some_and(|(t, _)| now.duration_since(*t) < GPU_TTL)
    {
        cache.gpus = Some((now, read_gpus(&cfg.nvidia_smi).await));
    }
    let llama = cache.llama.as_ref().map(|(_, l)| l.clone());
    let (gpus, gpus_error) = match cache.gpus.as_ref().map(|(_, g)| g.clone()) {
        Some(Ok(g)) => (g, None),
        Some(Err(e)) => (Vec::new(), Some(e)),
        None => (Vec::new(), None),
    };
    Ok(Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_s": state.0.started.elapsed().as_secs(),
        "llama": llama,
        "llama_origin": origin,
        "gpus": gpus,
        "gpus_error": gpus_error,
    })))
}
