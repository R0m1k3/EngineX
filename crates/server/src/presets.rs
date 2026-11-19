//! Presets : réglages de lancement d'un modèle local et réglages
//! d'échantillonnage lus sur la page Hugging Face du modèle.

use axum::{
    Json,
    extract::{Path as UrlPath, State},
};
use enginex_engine::args::{DraftKind, PresetSettings, Reasoning, needs_reload};
use enginex_hub::{
    fit::Verdict,
    gguf::GgufMeta,
    sampling::{self, Merged, SourceInputs},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    engine_api,
    error::{ApiError, ApiJson},
    models::{self, ModelRow},
    state::{AppState, now},
    system::{set_setting, setting},
};

/// Niveaux de `base_model` remontés au plus.
const BASE_DEPTH: usize = 2;
const NAME_MAX: usize = 120;

#[derive(sqlx::FromRow)]
pub struct PresetRow {
    pub id: i64,
    pub name: String,
    pub model_id: i64,
    pub settings: String,
    pub sources: String,
    pub updated_at: i64,
}

pub async fn preset(state: &AppState, id: i64) -> Result<PresetRow, ApiError> {
    sqlx::query_as("SELECT id, name, model_id, settings, sources, updated_at FROM presets WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.0.db)
        .await?
        .ok_or(ApiError::NotFound)
}

pub async fn active_id(state: &AppState) -> Result<Option<i64>, ApiError> {
    Ok(setting(state, "preset.active").await?.and_then(|v| v.parse().ok()))
}

/// Lit les sources d'échantillonnage d'un dépôt (card, base_model,
/// generation_config) ; les erreurs réseau sont gardées, pas renvoyées.
pub async fn read_sources(state: &AppState, repo: &str, meta: &GgufMeta) -> SourceInputs {
    let hf = &state.0.hf;
    let base = state.config().hf_base.trim_end_matches('/').to_string();
    let mut inputs = SourceInputs {
        gguf: sampling::from_gguf(meta),
        ..Default::default()
    };
    let mut errors = Vec::new();
    match hf.readme(repo).await {
        Ok(Some(md)) => inputs.card_repo = Some((format!("{base}/{repo}"), sampling::parse_card(&md))),
        Ok(None) => {}
        Err(e) => errors.push(format!("card de {repo} : {e}")),
    }
    let mut current = repo.to_string();
    for _ in 0..BASE_DEPTH {
        let b = match hf.base_model(&current).await {
            Ok(Some(b)) if models::valid_repo(&b) && b != current => b,
            Ok(_) => break,
            Err(e) => {
                errors.push(format!("base_model de {current} : {e}"));
                break;
            }
        };
        if inputs.card_base.as_ref().is_none_or(|(_, sets)| sets.is_empty()) {
            match hf.readme(&b).await {
                Ok(Some(md)) => inputs.card_base = Some((format!("{base}/{b}"), sampling::parse_card(&md))),
                Ok(None) => {}
                Err(e) => errors.push(format!("card de {b} : {e}")),
            }
        }
        if inputs.generation_config.is_none() {
            match hf.generation_config(&b).await {
                Ok(Some(v)) => {
                    inputs.generation_config = Some((
                        format!("{base}/{b}/blob/main/generation_config.json"),
                        sampling::from_generation_config(&v),
                    ))
                }
                Ok(None) => {}
                Err(e) => errors.push(format!("generation_config de {b} : {e}")),
            }
        }
        current = b;
    }
    if !errors.is_empty() {
        inputs.error = Some(errors.join(" ; "));
    }
    inputs
}

/// Preset créé à la fin d'un téléchargement.
pub async fn create_auto(state: &AppState, model_id: i64) -> anyhow::Result<i64> {
    let m = models::model(state, model_id)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let meta: GgufMeta = serde_json::from_str(&m.meta)?;
    let inputs = read_sources(state, &m.repo, &meta).await;
    let merged = sampling::merge(&inputs, true);
    let mut s = PresetSettings::default();
    if let Some(c) = meta.context_length {
        s.ctx = s.ctx.min(c.max(512));
    }
    s.sampling = merged.values.clone();
    s.mmproj = m.mmproj.is_some();
    // Couches MTP dans le GGUF : brouillon MTP actif d'office (désactivable).
    if meta.has_mtp() {
        s.spec.draft = DraftKind::Mtp;
    }
    // Modèle dense qui ne tient pas : contexte réduit (jusqu'à 16k) plutôt
    // que des couches calculées par le processeur.
    while s.ctx > 16384
        && let Ok(r) = models::local_fit(state, &m, &s).await
        && r.verdict == Verdict::NoFit
    {
        s.ctx = (s.ctx / 2).max(16384);
    }
    // MoE plus gros que la VRAM (experts en RAM) : lots plus grands, comme Loki.
    if let Ok(r) = models::local_fit(state, &m, &s).await
        && r.verdict == Verdict::Offload
    {
        s.batch = Some(4096);
        s.ubatch = Some(2048);
    }
    let quant = enginex_hub::hf::quant_of(&m.file);
    let short = m.repo.rsplit('/').next().unwrap_or(&m.repo).trim_end_matches("-GGUF");
    let t = now();
    let id = sqlx::query_scalar(
        "INSERT INTO presets (name, model_id, settings, sources, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(format!("{short} {quant}"))
    .bind(model_id)
    .bind(serde_json::to_string(&s)?)
    .bind(serde_json::to_string(&merged)?)
    .bind(t)
    .bind(t)
    .fetch_one(&state.0.db)
    .await?;
    Ok(id)
}

fn to_json(p: &PresetRow, active: Option<i64>) -> Result<Value, ApiError> {
    Ok(json!({
        "id": p.id, "name": p.name, "model_id": p.model_id,
        "settings": serde_json::to_value(serde_json::from_str::<PresetSettings>(&p.settings)?)?,
        "sources": serde_json::from_str::<Value>(&p.sources)?,
        "active": Some(p.id) == active,
    }))
}

pub async fn list(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let rows: Vec<PresetRow> =
        sqlx::query_as("SELECT id, name, model_id, settings, sources, updated_at FROM presets ORDER BY name, id")
            .fetch_all(&state.0.db)
            .await?;
    let active = active_id(&state).await?;
    Ok(Json(Value::Array(
        rows.iter().map(|p| to_json(p, active)).collect::<Result<_, _>>()?,
    )))
}

fn check_name(name: &str) -> Result<String, ApiError> {
    let n = name.trim();
    if n.is_empty() || n.chars().count() > NAME_MAX {
        return Err(ApiError::BadRequest);
    }
    Ok(n.to_string())
}

#[derive(Deserialize)]
pub struct CopyBody {
    from: i64,
    name: String,
}

pub async fn copy(State(state): State<AppState>, ApiJson(b): ApiJson<CopyBody>) -> Result<Json<Value>, ApiError> {
    let name = check_name(&b.name)?;
    let src = preset(&state, b.from).await?;
    let t = now();
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO presets (name, model_id, settings, sources, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(name)
    .bind(src.model_id)
    .bind(&src.settings)
    .bind(&src.sources)
    .bind(t)
    .bind(t)
    .fetch_one(&state.0.db)
    .await?;
    Ok(Json(json!({ "id": id })))
}

/// MTP seulement si le GGUF a des couches MTP ; brouillon = un autre modèle
/// local.
async fn check_accel(state: &AppState, model_id: i64, s: &PresetSettings) -> Result<(), ApiError> {
    match s.spec.draft {
        DraftKind::Mtp => {
            let m = models::model(state, model_id).await?;
            let meta: GgufMeta = serde_json::from_str(&m.meta)?;
            if !meta.has_mtp() {
                return Err(ApiError::BadRequest);
            }
        }
        DraftKind::Model => {
            let did = s.spec.draft_model_id.ok_or(ApiError::BadRequest)?;
            if did == model_id || models::model(state, did).await.is_err() {
                return Err(ApiError::BadRequest);
            }
        }
        DraftKind::None => {}
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct UpdateBody {
    name: String,
    settings: PresetSettings,
}

pub async fn update(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<i64>,
    ApiJson(b): ApiJson<UpdateBody>,
) -> Result<Json<Value>, ApiError> {
    let name = check_name(&b.name)?;
    if b.settings.invalid().is_some() || b.settings.extra_args.iter().any(|a| a.len() > 1000) {
        return Err(ApiError::BadRequest);
    }
    let old = preset(&state, id).await?;
    check_accel(&state, old.model_id, &b.settings).await?;
    let old_s: PresetSettings = serde_json::from_str(&old.settings)?;
    let reload_needed = active_id(&state).await? == Some(id) && needs_reload(&old_s, &b.settings);
    sqlx::query("UPDATE presets SET name = ?, settings = ?, updated_at = ? WHERE id = ?")
        .bind(name)
        .bind(serde_json::to_string(&b.settings)?)
        .bind(now())
        .bind(id)
        .execute(&state.0.db)
        .await?;
    Ok(Json(json!({ "reload_needed": reload_needed })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FitBody {
    settings: PresetSettings,
}

/// Place estimée du modèle d'un preset avec des réglages en cours d'édition.
pub async fn fit(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<i64>,
    ApiJson(b): ApiJson<FitBody>,
) -> Result<Json<enginex_hub::fit::FitReport>, ApiError> {
    if b.settings.invalid().is_some() {
        return Err(ApiError::BadRequest);
    }
    let p = preset(&state, id).await?;
    let m: ModelRow = models::model(&state, p.model_id).await?;
    Ok(Json(models::local_fit(&state, &m, &b.settings).await?))
}

async fn delete_row(state: &AppState, id: i64) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM presets WHERE id = ?")
        .bind(id)
        .execute(&state.0.db)
        .await
        .map(|_| ())
}

/// Preset inactif : supprimé tout de suite. Preset actif : tâche en fond qui
/// attend la fin des requêtes /v1, arrête le moteur, puis supprime.
pub async fn delete(State(state): State<AppState>, UrlPath(id): UrlPath<i64>) -> Result<Json<Value>, ApiError> {
    preset(&state, id).await?;
    if active_id(&state).await? != Some(id) {
        delete_row(&state, id).await?;
        return Ok(Json(json!({ "ok": true })));
    }
    engine_api::begin_job(&state, "delete", &id.to_string())?;
    let s = state.clone();
    tokio::spawn(async move {
        let _gate = engine_api::take_gate(&s).await;
        s.supervisor().stop(false).await;
        let r = async {
            set_setting(&s, "preset.active", None).await?;
            delete_row(&s, id).await
        }
        .await
        .map(|_| "preset supprimé, moteur arrêté".to_string())
        .map_err(|e| e.to_string());
        engine_api::end_job(&s, r);
    });
    Ok(Json(json!({ "started": true })))
}

#[derive(Deserialize)]
pub struct ActivateBody {
    #[serde(default)]
    confirm_nofit: bool,
}

pub async fn activate(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<i64>,
    ApiJson(b): ApiJson<ActivateBody>,
) -> Result<Json<Value>, ApiError> {
    let p = preset(&state, id).await?;
    let s: PresetSettings = serde_json::from_str(&p.settings)?;
    let m: ModelRow = models::model(&state, p.model_id).await?;
    let report = models::local_fit(&state, &m, &s).await?;
    if report.verdict == Verdict::NoFit && !b.confirm_nofit {
        return Err(ApiError::NofitConfirmRequired);
    }
    engine_api::begin_job(&state, "activate", &id.to_string())?;
    let s = state.clone();
    tokio::spawn(async move {
        // Porte prise avant de changer de preset : les requêtes en cours
        // finissent sur l'ancien, les suivantes attendent le nouveau.
        let _gate = engine_api::take_gate(&s).await;
        let r = async {
            set_setting(&s, "preset.active", Some(&id.to_string()))
                .await
                .map_err(|e| e.to_string())?;
            engine_api::launch(&s, id).await.map_err(|e| e.to_string())?;
            if engine_api::wait_loaded(&s).await {
                Ok(format!("preset « {} » chargé", p.name))
            } else {
                Err(match s.supervisor().state() {
                    enginex_engine::supervisor::EngineState::Failed { cause, .. } => cause,
                    _ => "chargement échoué".into(),
                })
            }
        }
        .await;
        engine_api::end_job(&s, r);
    });
    Ok(Json(json!({ "started": true, "fit": report })))
}

pub async fn refresh_sources(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<i64>,
) -> Result<Json<Value>, ApiError> {
    let p = preset(&state, id).await?;
    let m = models::model(&state, p.model_id).await?;
    let meta: GgufMeta = serde_json::from_str(&m.meta)?;
    let mut s: PresetSettings = serde_json::from_str(&p.settings)?;
    let inputs = read_sources(&state, &m.repo, &meta).await;
    let merged: Merged = sampling::merge(&inputs, s.reasoning == Reasoning::On);
    s.sampling = merged.values.clone();
    sqlx::query("UPDATE presets SET settings = ?, sources = ?, updated_at = ? WHERE id = ?")
        .bind(serde_json::to_string(&s)?)
        .bind(serde_json::to_string(&merged)?)
        .bind(now())
        .bind(id)
        .execute(&state.0.db)
        .await?;
    let p = preset(&state, id).await?;
    Ok(Json(to_json(&p, active_id(&state).await?)?))
}
