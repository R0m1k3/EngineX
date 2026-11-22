//! EngineX : serveur de l'API et de l'UI embarquée.

pub mod agents;
pub mod api_keys;
pub mod appgate;
pub mod attachments;
pub mod auth;
pub mod autoupdate;
pub mod bench;
pub mod board;
pub mod bridge;
pub mod browser;
pub mod client_ip;
pub mod codex_config;
pub mod config;
pub mod copies;
pub mod crons;
pub mod db;
pub mod engine_api;
pub mod error;
pub mod feed;
pub mod files;
pub mod forge;
pub mod git;
pub mod limiter;
pub mod mask;
pub mod mcp;
pub mod memory;
pub mod models;
pub mod presets;
pub mod preview;
pub mod queue;
pub mod recall;
pub mod sandboxes;
pub mod sessions;
pub mod skills;
pub mod snapshots;
pub mod state;
pub mod system;
pub mod terminal;
pub mod v1;
pub mod vault;
pub mod verify;
pub mod web;
pub mod webtools;
pub mod workspace;
pub mod wsfiles;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Request},
    http::{HeaderValue, Method, header},
    middleware,
    response::{IntoResponse, Response},
    routing::{delete, get, patch, post, put},
};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::error::{ApiError, V1Error};
pub use crate::{config::Config, state::AppState};

const CSP: &str = "default-src 'self'; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; frame-src 'self' http: https:; frame-ancestors 'none'";

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
        .route("/api/presets/{id}/fit", post(presets::fit))
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
        .route("/api/memory", get(memory::tree))
        .route("/api/memory/file", get(memory::get_file).put(memory::put_file))
        .route("/api/memory/revisions", get(memory::revisions))
        .route("/api/memory/revisions/{id}", get(memory::revision))
        .route("/api/tasks", get(board::list).post(board::create))
        .route("/api/tasks/projects", post(board::create_project))
        .route("/api/tasks/projects/{id}", delete(board::delete_project))
        .route("/api/tasks/{id}", patch(board::patch).delete(board::delete))
        .route("/api/crons", get(crons::list).post(crons::create))
        .route("/api/crons/preview", post(crons::preview))
        .route("/api/crons/{id}", put(crons::update).delete(crons::delete))
        .route("/api/crons/{id}/run", post(crons::run_now))
        .route("/api/crons/{id}/runs", get(crons::runs))
        .route("/api/agents", get(agents::list).post(agents::create))
        .route("/api/agents/{id}", put(agents::update).delete(agents::delete))
        .route("/api/agents/{id}/run", post(agents::run_now))
        .route("/api/agents/{id}/stop", post(agents::stop))
        .route("/api/agents/{id}/runs", get(agents::runs))
        .route("/api/bench/cases", get(bench::list_cases))
        .route("/api/bench/runs", get(bench::list_runs).post(bench::start))
        .route("/api/bench/runs/{id}", get(bench::get_run).delete(bench::delete_run))
        .route("/api/bench/runs/{id}/stop", post(bench::stop))
        .route("/api/bench/runs/{id}/cases/{cid}/rate", post(bench::rate))
        .route("/api/bench/compare", get(bench::compare))
        .route("/api/skills", get(skills::list))
        .route("/api/skills/{name}", get(skills::get).put(skills::put))
        .route("/api/skills/{name}/enabled", put(skills::put_enabled))
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
        .route("/api/sessions/{id}/files/delete", post(files::delete))
        .route("/api/sessions/{id}/files/zip", get(files::zip))
        .route("/api/sessions/{id}/turns/{tid}/diff", get(sessions::turn_diff))
        .route("/api/sessions/{id}/turns/{tid}/undo-file", post(sessions::undo_file))
        .route("/api/sessions/{id}/turns/{tid}/revert", post(sessions::revert))
        .route("/api/sessions/{id}/restore-undo", post(sessions::restore_undo))
        .route("/api/sessions/{id}/restore-green", post(sessions::restore_green))
        .route("/api/sessions/{id}/bring-back", post(copies::bring_back))
        .route("/api/sessions/{id}/terminal", get(terminal::ws))
        .route("/api/sessions/{id}/verify", get(verify::get))
        .route("/api/sessions/{id}/verify/save", post(verify::save))
        .route("/api/sessions/{id}/verify/run", post(verify::run_now))
        .route("/api/sessions/{id}/events", get(sessions::events))
        .route("/api/files/tree", get(wsfiles::tree))
        .route("/api/files/raw", get(wsfiles::raw))
        .route("/api/files/text", get(wsfiles::get_text).put(wsfiles::put_text))
        .route(
            "/api/files/upload",
            post(wsfiles::upload).layer(DefaultBodyLimit::max(wsfiles::UPLOAD_LIMIT)),
        )
        .route("/api/files/mkdir", post(wsfiles::mkdir))
        .route("/api/files/move", post(wsfiles::rename))
        .route("/api/files/delete", post(wsfiles::delete))
        .route("/api/files/zip", get(wsfiles::zip))
        .route("/api/files/usage", get(wsfiles::usage))
        .route("/api/git/repos", get(git::repos))
        .route("/api/git/repos/{id}", get(git::repo))
        .route("/api/git/repos/{id}/commits/{sha}", get(git::commit))
        .route("/api/git/repos/{id}/fetch", post(git::fetch))
        .route("/api/git/repos/{id}/pull", post(git::pull))
        .route("/api/git/repos/{id}/push", post(git::push))
        .route("/api/git/repos/{id}/checkout", post(git::checkout))
        .route("/api/git/repos/{id}/merge", post(git::merge))
        .route("/api/git/repos/{id}/branches", post(git::create_branch))
        .route("/api/git/repos/{id}/pulls", get(git::pulls).post(git::pull_create))
        .route("/api/git/repos/{id}/pulls/{n}", get(git::pull_detail))
        .route("/api/git/repos/{id}/pulls/{n}/comments", post(git::pull_comment))
        .route("/api/git/repos/{id}/pulls/{n}/merge", post(git::pull_merge))
        .route("/api/sessions/{id}/preview", get(preview::status).post(preview::open))
        .route("/api/sessions/{id}/browser/live", get(browser::live))
        .route("/api/browser", get(browser::status))
        .route("/api/queue", get(queue::api))
        .route("/api/feed", get(feed::list).delete(feed::clear))
        .route("/api/feed/read", post(feed::mark_read))
        .route("/api/feed/events", get(feed::events))
        .route("/api/feed/{id}", delete(feed::delete_one))
        .route("/api/push", get(feed::push_status))
        .route("/api/push/subscribe", post(feed::subscribe))
        .route("/api/push/unsubscribe", post(feed::unsubscribe))
        .route("/api/push/mode", put(feed::set_mode))
        .route("/api/push/test", post(feed::push_test))
        .route("/api/webhooks", get(feed::hooks_list).post(feed::hooks_create))
        .route(
            "/api/webhooks/{id}",
            patch(feed::hooks_patch).delete(feed::hooks_delete),
        )
        .route("/api/sessions/{id}/git", get(git::session_status))
        .route("/api/sessions/{id}/git/commit", post(git::session_commit))
        .route("/api/sessions/{id}/git/push", post(git::session_push))
        .route("/api/sessions/{id}/git/pull-request", post(git::session_pull_request))
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
    // `/v1` et `/mcp` hors des couches de l'UI (session, même origine) : seule
    // la clé d'API les protège. Le seul fallback est celui de `ui`.
    // Webhooks entrants : le jeton de l'adresse est le seul secret.
    let hooks = Router::new()
        .route("/hooks/{token}", post(feed::hook_in))
        .layer(DefaultBodyLimit::max(feed::HOOK_BODY_MAX));
    Router::new()
        .merge(hooks)
        .merge(v1::router(&state))
        .merge(mcp::router(&state))
        .merge(ui)
        .with_state(state)
}
