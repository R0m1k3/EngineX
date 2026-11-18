//! EngineX : serveur de l'API et de l'UI embarquée.

pub mod auth;
pub mod client_ip;
pub mod config;
pub mod db;
pub mod error;
pub mod limiter;
pub mod state;
pub mod system;
pub mod web;

use axum::{
    Json, Router,
    extract::Request,
    http::{HeaderValue, Method, header},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use tower_http::set_header::SetResponseHeaderLayer;

use crate::error::ApiError;
pub use crate::{config::Config, state::AppState};

const CSP: &str =
    "default-src 'self'; img-src 'self' data: blob:; style-src 'self' 'unsafe-inline'; frame-ancestors 'none'";

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }))
}

/// Hors `/api` : fichiers de l'UI, sinon 405 (méthode) ou 404.
async fn fallback(req: Request) -> Response {
    let path = req.uri().path();
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
        .route_layer(middleware::from_fn_with_state(state.clone(), auth::require_session));
    let public = Router::new()
        .route("/api/health", get(health))
        .route("/api/auth/state", get(auth::auth_state))
        .route("/api/auth/setup", post(auth::setup))
        .route("/api/auth/login", post(auth::login));
    Router::new()
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
        ))
        .with_state(state)
}
