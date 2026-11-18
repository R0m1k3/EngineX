//! EngineX : serveur de l'API et de l'UI embarquée.

pub mod api_keys;
pub mod attachments;
pub mod auth;
pub mod autoupdate;
pub mod bridge;
pub mod client_ip;
pub mod codex_config;
pub mod config;
pub mod db;
pub mod engine_api;
pub mod error;
pub mod files;
pub mod limiter;
pub mod mask;
pub mod models;
pub mod presets;
pub mod sandboxes;
pub mod sessions;
pub mod snapshots;
pub mod state;
pub mod system;
pub mod terminal;
pub mod v1;
pub mod vault;
pub mod verify;
pub mod web;
pub mod workspace;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request},
    http::{HeaderValue, Method, header},
    middleware,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::error::{ApiError, V1Error};
pub use crate::{config::Config, state::AppState};

const CSP: &str =
    "default-src 'self'; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; frame-ancestors 'none'";

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }))
}

/// Hors `/api` : fichiers de l'UI, sinon 405 (méthode) ou 404.
async fn fallback(req: Request) -> Response {
    let path = req.uri().path();
    if path == "/v1" || path.starts_with("/v1/") {
        return V1Error::not_found().into_response();
    }
    if path == "/api" || path.starts_with("/api/") {
        return ApiError::NotFound.into_response();
    }
    if let Some(res) = web::serve(req.method(), path, req.headers()) {
        return res;
    }
    if req.method() != Method::GET && req.method() != Method::HEAD {
        ApiError::MethodNotAllowed.into_response()
    } else {
        ApiError::NotFound.into_response()
    }
}

pub fn app(state: AppState) -> Router {
    let private = Router::new()
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/auth/password", post(auth::change_password))
        .route("/api/system", get(system::system))
        .route("/api/hf/search", get(models::search))
        .route("/api/hf/repo", get(models::repo))
        .route("/api/hf/fit", get(models::fit_route))
        .route(
            "/api/downloads",
            get(models::list_downloads).post(models::create_download),
        )
        .route("/api/downloads/{id}", delete(models::delete_download))
        .route("/api/downloads/{id}/pause", post(models::pause_download))
        .route("/api/downloads/{id}/resume", post(models::resume_download))
        .route("/api/models", get(models::list_models))
        .route("/api/models/{id}", delete(models::delete_model))
        .route("/api/presets", get(presets::list).post(presets::copy))
        .route("/api/presets/{id}", put(presets::update).delete(presets::delete))
        .route("/api/presets/{id}/activate", post(presets::activate))
        .route("/api/presets/{id}/refresh-sources", post(presets::refresh_sources))
        .route("/api/engine", get(engine_api::engine))
        .route("/api/engine/stop", post(engine_api::stop))
        .route("/api/engine/auto-update", put(autoupdate::set_auto))
        .route("/api/engine/logs", get(engine_api::logs))
        .route("/api/engine/versions", get(engine_api::versions))
        .route("/api/engine/versions/check", post(engine_api::check))
        .route("/api/engine/versions/install", post(engine_api::install))
        .route("/api/engine/versions/rollback", post(engine_api::rollback))
        .route("/api/engine/versions/{tag}", delete(engine_api::delete_version))
        .route("/api/engine/versions/{tag}/use", post(engine_api::use_version))
        .route("/api/api-keys", get(api_keys::list).post(api_keys::create))
        .route("/api/api-keys/{id}", delete(api_keys::delete))
        .route("/api/codex-config", get(codex_config::codex_config))
        .route("/api/vault", get(vault::list).put(vault::put))
        .route("/api/vault/{id}", delete(vault::delete))
        .route("/api/projects", get(workspace::list).post(workspace::create_project))
        .route("/api/projects/{id}", delete(workspace::delete_project))
        .route("/api/projects/{id}/topics", post(workspace::create_topic))
        .route("/api/topics/{id}", delete(workspace::delete_topic))
        .route("/api/sessions", get(sessions::list).post(sessions::create))
        .route(
            "/api/sessions/{id}",
            get(sessions::get).patch(sessions::patch).delete(sessions::delete),
        )
        .route("/api/sessions/{id}/turns", post(sessions::post_turn))
        .route("/api/sessions/{id}/interrupt", post(sessions::interrupt))
        .route("/api/sessions/{id}/requests/{rid}", post(sessions::answer))
        .route(
            "/api/sessions/{id}/attachments",
            post(attachments::upload).layer(DefaultBodyLimit::max(attachments::BODY_LIMIT)),
        )
        .route("/api/sessions/{id}/attachments/{name}", get(attachments::serve))
        .route("/api/sessions/{id}/file", get(files::file))
        .route("/api/sessions/{id}/file/text", get(files::text))
        .route("/api/sessions/{id}/tree", get(files::tree))
        .route("/api/sessions/{id}/turns/{tid}/diff", get(sessions::turn_diff))
        .route("/api/sessions/{id}/turns/{tid}/undo-file", post(sessions::undo_file))
        .route("/api/sessions/{id}/turns/{tid}/revert", post(sessions::revert))
        .route("/api/sessions/{id}/restore-undo", post(sessions::restore_undo))
        .route("/api/sessions/{id}/restore-green", post(sessions::restore_green))
        .route("/api/sessions/{id}/terminal", get(terminal::ws))
        .route("/api/sessions/{id}/verify", get(verify::get))
        .route("/api/sessions/{id}/verify/save", post(verify::save))
        .route("/api/sessions/{id}/verify/run", post(verify::run_now))
        .route("/api/sessions/{id}/events", get(sessions::events))
        .route("/api/sandbox/status", get(sandboxes::status))
        .route(
            "/api/settings/sessions",
            get(sandboxes::get_settings).put(sandboxes::put_settings),
        )
        .route_layer(middleware::from_fn_with_state(state.clone(), auth::require_session));
    let public = Router::new()
        .route("/api/health", get(health))
        .route("/api/auth/state", get(auth::auth_state))
        .route("/api/auth/setup", post(auth::setup))
        .route("/api/auth/login", post(auth::login));
    let ui = Router::new()
        .merge(public)
        .merge(private)
        .fallback(fallback)
        .layer(middleware::from_fn_with_state(state.clone(), auth::same_origin))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(CSP),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::REFERRER_POLICY,
            HeaderValue::from_static("same-origin"),
        ));
    // `/v1` hors des couches de l'UI (session, même origine) : seule la clé
    // d'API le protège. Le seul fallback est celui de `ui`.
    Router::new().merge(v1::router(&state)).merge(ui).with_state(state)
}
