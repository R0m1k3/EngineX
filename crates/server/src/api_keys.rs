//! Clés d'API de `/v1` : plusieurs clés nommées, seul leur SHA-256 est en
//! base, la valeur ne s'affiche qu'une fois (à la création).

use axum::{
    Json,
    extract::{Path as UrlPath, State},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    auth::{random_token, sha256},
    error::{ApiError, ApiJson},
    state::{AppState, now},
};

const NAME_MAX: usize = 50;
pub const PREFIX: &str = "ex_";

pub async fn list(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let rows: Vec<(i64, String, i64, Option<i64>)> = sqlx::query_as(
        "SELECT id, name, created_at, last_used_at FROM api_keys WHERE internal = 0 ORDER BY created_at, id",
    )
    .fetch_all(state.db())
    .await?;
    Ok(Json(Value::Array(
        rows.into_iter()
            .map(|(id, name, c, u)| json!({"id": id, "name": name, "created_at": c, "last_used_at": u}))
            .collect(),
    )))
}

#[derive(Deserialize)]
pub struct CreateBody {
    name: String,
}

/// Crée une clé ; seule réponse qui contient sa valeur.
pub async fn create(State(state): State<AppState>, ApiJson(b): ApiJson<CreateBody>) -> Result<Json<Value>, ApiError> {
    let name = b.name.trim();
    if name.is_empty() || name.chars().count() > NAME_MAX {
        return Err(ApiError::BadRequest);
    }
    let key = format!("{PREFIX}{}", random_token());
    let t = now();
    let res: Result<i64, sqlx::Error> =
        sqlx::query_scalar("INSERT INTO api_keys (name, hash, created_at) VALUES (?, ?, ?) RETURNING id")
            .bind(name)
            .bind(sha256(key.as_bytes()))
            .bind(t)
            .fetch_one(state.db())
            .await;
    let id = match res {
        Ok(id) => id,
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => return Err(ApiError::NameTaken),
        Err(e) => return Err(e.into()),
    };
    tracing::info!("clé d'API « {name} » créée");
    Ok(Json(json!({"id": id, "name": name, "key": key, "created_at": t})))
}

pub async fn delete(State(state): State<AppState>, UrlPath(id): UrlPath<i64>) -> Result<Json<Value>, ApiError> {
    // Les clés `internal` (sandboxes) n'existent pas pour l'UI.
    let done = sqlx::query("DELETE FROM api_keys WHERE id = ? AND internal = 0")
        .bind(id)
        .execute(state.db())
        .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(Json(json!({ "ok": true })))
}

/// `(id, internal)` de la clé, si elle existe (recherche du hash) ; pose
/// `last_used_at`.
pub(crate) async fn find(state: &AppState, key: &str) -> Result<Option<(i64, bool)>, sqlx::Error> {
    sqlx::query_as("UPDATE api_keys SET last_used_at = ? WHERE hash = ? RETURNING id, internal")
        .bind(now())
        .bind(sha256(key.as_bytes()))
        .fetch_optional(state.db())
        .await
}

/// Crée la clé interne `name` (sandbox) et rend sa valeur.
pub(crate) async fn create_internal(state: &AppState, name: &str) -> Result<String, sqlx::Error> {
    let key = format!("{PREFIX}{}", random_token());
    sqlx::query("INSERT INTO api_keys (name, hash, created_at, internal) VALUES (?, ?, ?, 1)")
        .bind(name)
        .bind(sha256(key.as_bytes()))
        .bind(now())
        .execute(state.db())
        .await?;
    Ok(key)
}

/// Révoque la clé interne `name` (sans erreur si absente).
pub(crate) async fn revoke_internal(state: &AppState, name: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM api_keys WHERE name = ? AND internal = 1")
        .bind(name)
        .execute(state.db())
        .await
        .map(|_| ())
}
