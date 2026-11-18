//! Coffre de secrets : valeurs chiffrées (XChaCha20-Poly1305) par la clé
//! maîtresse `<data>/master.key` (32 octets, droits 600). Données associées =
//! `scope || 0x00 || name` : un chiffré ne peut pas changer de nom.
//!
//! Les valeurs ne quittent le coffre que par `env_for` (environnement d'une
//! sandbox) et `all_values` (masquage).

use std::{io::Write as _, os::unix::fs::OpenOptionsExt, path::Path, sync::RwLock};

use axum::{
    Json,
    extract::{Path as UrlPath, State},
};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use rand::RngCore;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    error::{ApiError, ApiJson},
    state::{AppState, now},
};

pub const KEY_FILE: &str = "master.key";
const VALUE_MAX: usize = 16 * 1024;
const RESERVED: &[&str] = &["HOME", "PATH", "USER"];

#[derive(Default)]
pub struct Vault {
    key: RwLock<Option<[u8; 32]>>,
}

fn write_key(path: &Path) -> anyhow::Result<[u8; 32]> {
    let mut key = [0u8; 32];
    rand::rng().fill_bytes(&mut key);
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(&key)?;
    f.sync_all()?;
    std::fs::rename(&tmp, path)?;
    Ok(key)
}

fn read_key(path: &Path) -> Option<[u8; 32]> {
    std::fs::read(path).ok()?.try_into().ok()
}

fn aad(scope: &str, name: &str) -> Vec<u8> {
    let mut a = scope.as_bytes().to_vec();
    a.push(0);
    a.extend_from_slice(name.as_bytes());
    a
}

pub fn encrypt(key: &[u8; 32], scope: &str, name: &str, value: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let cipher = XChaCha20Poly1305::new(key.into());
    let mut nonce = [0u8; 24];
    rand::rng().fill_bytes(&mut nonce);
    let aad = aad(scope, name);
    let ct = cipher
        .encrypt(&XNonce::from(nonce), Payload { msg: value, aad: &aad })
        .expect("chiffrement");
    (nonce.to_vec(), ct)
}

pub fn decrypt(key: &[u8; 32], scope: &str, name: &str, nonce: &[u8], ct: &[u8]) -> Option<Vec<u8>> {
    let nonce: [u8; 24] = nonce.try_into().ok()?;
    let cipher = XChaCha20Poly1305::new(key.into());
    let aad = aad(scope, name);
    cipher
        .decrypt(&XNonce::from(nonce), Payload { msg: ct, aad: &aad })
        .ok()
}

/// Nom de variable d'environnement permis.
pub fn valid_name(name: &str) -> bool {
    let re_ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .enumerate()
            .all(|(i, c)| c == '_' || c.is_ascii_uppercase() || (i > 0 && c.is_ascii_digit()));
    re_ok && !name.starts_with("ENGINEX_") && !name.starts_with("CODEX_") && !RESERVED.contains(&name)
}

impl Vault {
    /// Ouvre la clé maîtresse ; la crée seulement s'il n'y a aucun secret.
    pub async fn open(state: &AppState) -> anyhow::Result<()> {
        let path = state.config().data_dir.join(KEY_FILE);
        let key = match read_key(&path) {
            Some(k) => Some(k),
            None => {
                let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM secrets")
                    .fetch_one(state.db())
                    .await?;
                if n > 0 {
                    tracing::error!(
                        "coffre verrouillé : {} absent ou invalide alors que {n} secret(s) existent (restaurer la clé ou supprimer les secrets)",
                        path.display()
                    );
                    None
                } else {
                    Some(write_key(&path)?)
                }
            }
        };
        *state.0.vault.key.write().expect("verrou") = key;
        Ok(())
    }

    pub fn locked(&self) -> bool {
        self.key.read().expect("verrou").is_none()
    }

    fn key(&self) -> Option<[u8; 32]> {
        *self.key.read().expect("verrou")
    }
}

type Row = (String, String, Vec<u8>, Vec<u8>);

async fn rows(state: &AppState, scopes: &[String]) -> Result<Vec<Row>, sqlx::Error> {
    let mut out = Vec::new();
    for s in scopes {
        let mut r: Vec<Row> =
            sqlx::query_as("SELECT scope, name, nonce, value FROM secrets WHERE scope = ? ORDER BY name")
                .bind(s)
                .fetch_all(state.db())
                .await?;
        out.append(&mut r);
    }
    Ok(out)
}

/// Seul point où des valeurs quittent le coffre pour une sandbox : secrets
/// `global` puis `project:<id>` (le projet l'emporte à nom égal).
pub async fn env_for(state: &AppState, project_id: i64) -> Result<Vec<(String, String)>, sqlx::Error> {
    let Some(key) = state.0.vault.key() else {
        return Ok(Vec::new());
    };
    let mut map = std::collections::BTreeMap::new();
    for (scope, name, nonce, ct) in rows(state, &["global".into(), format!("project:{project_id}")]).await? {
        match decrypt(&key, &scope, &name, &nonce, &ct) {
            Some(v) => {
                map.insert(name, String::from_utf8_lossy(&v).into_owned());
            }
            None => tracing::warn!("secret {scope}/{name} indéchiffrable"),
        }
    }
    Ok(map.into_iter().collect())
}

/// Toutes les valeurs (masquage).
pub async fn all_values(state: &AppState) -> Result<Vec<String>, sqlx::Error> {
    let Some(key) = state.0.vault.key() else {
        return Ok(Vec::new());
    };
    let r: Vec<Row> = sqlx::query_as("SELECT scope, name, nonce, value FROM secrets")
        .fetch_all(state.db())
        .await?;
    Ok(r.into_iter()
        .filter_map(|(s, n, nonce, ct)| decrypt(&key, &s, &n, &nonce, &ct))
        .map(|v| String::from_utf8_lossy(&v).into_owned())
        .collect())
}

// ---- Routes ----

pub async fn list(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let r: Vec<(i64, String, String, i64, i64)> =
        sqlx::query_as("SELECT id, scope, name, created_at, updated_at FROM secrets ORDER BY scope, name")
            .fetch_all(state.db())
            .await?;
    let secrets: Vec<Value> = r
        .into_iter()
        .map(
            |(id, scope, name, c, u)| json!({"id": id, "scope": scope, "name": name, "created_at": c, "updated_at": u}),
        )
        .collect();
    Ok(Json(json!({"secrets": secrets, "locked": state.0.vault.locked()})))
}

#[derive(Deserialize)]
pub struct PutBody {
    scope: String,
    name: String,
    value: String,
}

async fn valid_scope(state: &AppState, scope: &str) -> Result<bool, ApiError> {
    if scope == "global" {
        return Ok(true);
    }
    let Some(id) = scope.strip_prefix("project:").and_then(|i| i.parse::<i64>().ok()) else {
        return Ok(false);
    };
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM projects WHERE id = ?")
        .bind(id)
        .fetch_one(state.db())
        .await?;
    Ok(n == 1)
}

pub async fn put(State(state): State<AppState>, ApiJson(b): ApiJson<PutBody>) -> Result<Json<Value>, ApiError> {
    let Some(key) = state.0.vault.key() else {
        return Err(ApiError::VaultLocked);
    };
    if !valid_name(&b.name) {
        return Err(ApiError::InvalidName);
    }
    if !valid_scope(&state, &b.scope).await? {
        return Err(ApiError::BadRequest);
    }
    if b.value.is_empty() || b.value.len() > VALUE_MAX {
        return Err(ApiError::BadRequest);
    }
    let (nonce, ct) = encrypt(&key, &b.scope, &b.name, b.value.as_bytes());
    let t = now();
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO secrets (scope, name, nonce, value, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?) \
         ON CONFLICT(scope, name) DO UPDATE SET nonce = excluded.nonce, value = excluded.value, \
         updated_at = excluded.updated_at RETURNING id",
    )
    .bind(&b.scope)
    .bind(&b.name)
    .bind(nonce)
    .bind(ct)
    .bind(t)
    .bind(t)
    .fetch_one(state.db())
    .await?;
    tracing::info!("secret {}/{} enregistré", b.scope, b.name);
    crate::sandboxes::rebuild_masker(&state).await;
    Ok(Json(json!({ "id": id })))
}

pub async fn delete(State(state): State<AppState>, UrlPath(id): UrlPath<i64>) -> Result<Json<Value>, ApiError> {
    let done = sqlx::query("DELETE FROM secrets WHERE id = ?")
        .bind(id)
        .execute(state.db())
        .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    // Coffre verrouillé et vidé : nouvelle clé maîtresse.
    if state.0.vault.locked() {
        Vault::open(&state).await?;
    }
    crate::sandboxes::rebuild_masker(&state).await;
    Ok(Json(json!({ "ok": true })))
}
