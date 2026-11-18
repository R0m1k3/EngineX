//! Authentification mono-utilisateur : code de configuration, mot de passe
//! argon2id, sessions en cookie (seul le SHA-256 du jeton est en base),
//! limiteur par IP et contrôle de l'origine.

use std::{net::SocketAddr, time::Instant};

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use axum::{
    Json,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, HeaderValue, Method, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::Rng;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::{
    client_ip::{client_ip, limiter_key},
    error::{ApiError, ApiJson},
    state::AppState,
};

pub const COOKIE: &str = "enginex_session";
pub const SESSION_SECS: i64 = 30 * 24 * 3600;
pub const MIN_PASSWORD: usize = 12;
const CODE_ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/// Empreinte du jeton de la session courante, posée par `require_session`.
#[derive(Clone)]
pub struct SessionId(pub Vec<u8>);

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub(crate) fn sha256(data: &[u8]) -> Vec<u8> {
    Sha256::digest(data).to_vec()
}

/// Code lisible `XXXX-XXXX-XXXX` (12 caractères base32 de Crockford).
fn new_code() -> String {
    let mut rng = rand::rng();
    let chars: Vec<char> = (0..12)
        .map(|_| CODE_ALPHABET[rng.random_range(0..CODE_ALPHABET.len())] as char)
        .collect();
    chars
        .chunks(4)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("-")
}

/// Le code tel que tapé : majuscules, sans tirets ni espaces.
fn normalize_code(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

async fn password_hash(state: &AppState) -> Result<Option<String>, ApiError> {
    Ok(
        sqlx::query_scalar("SELECT value FROM settings WHERE key = 'password_hash'")
            .fetch_optional(state.db())
            .await?,
    )
}

/// Vrai si la configuration reste à faire ; crée alors le code s'il manque
/// et l'écrit dans les logs.
pub async fn ensure_setup_code(state: &AppState) -> Result<bool, ApiError> {
    let missing = password_hash(state).await?.is_none();
    let mut code = state.0.setup_code.lock().expect("verrou");
    if !missing {
        *code = None;
        return Ok(false);
    }
    if code.is_none() {
        let c = new_code();
        tracing::warn!("Code de configuration EngineX : {c}");
        *code = Some(c);
    }
    Ok(true)
}

/// IP utilisée par le limiteur.
pub(crate) fn ip_of(state: &AppState, peer: &SocketAddr, headers: &HeaderMap) -> String {
    let raw = if state.config().trust_proxy {
        let xff = headers.get("x-forwarded-for").and_then(|v| v.to_str().ok());
        client_ip(peer, xff)
    } else {
        peer.ip().to_string()
    };
    limiter_key(raw)
}

fn check_limiter(state: &AppState, ip: &str) -> Result<(), ApiError> {
    let wait = state.0.limiter.lock().expect("verrou").locked_for(ip, Instant::now());
    if wait.is_zero() {
        Ok(())
    } else {
        Err(ApiError::RateLimited(wait.as_secs().max(1)))
    }
}

fn record_failure(state: &AppState, ip: &str) {
    state
        .0
        .limiter
        .lock()
        .expect("verrou")
        .record_failure(ip, Instant::now());
}

fn clear_failures(state: &AppState, ip: &str) {
    state.0.limiter.lock().expect("verrou").clear(ip);
}

/// Calcul argon2 dans un fil bloquant, au plus `HASH_PERMITS` à la fois.
async fn with_hash_permit<T: Send + 'static>(
    state: &AppState,
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, ApiError> {
    let _permit = state
        .0
        .hash_permits
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::Busy)?;
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("calcul argon2 : {e}")))
}

async fn hash_password(state: &AppState, password: String) -> Result<String, ApiError> {
    with_hash_permit(state, move || {
        let mut salt_bytes = [0u8; 16];
        rand::fill(&mut salt_bytes);
        let salt = SaltString::encode_b64(&salt_bytes).map_err(|e| anyhow::anyhow!("sel : {e}"))?;
        Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map(|h| h.to_string())
            .map_err(|e| anyhow::anyhow!("argon2 : {e}"))
    })
    .await?
    .map_err(ApiError::Internal)
}

async fn verify_password(state: &AppState, password: String, hash: String) -> Result<bool, ApiError> {
    with_hash_permit(state, move || {
        PasswordHash::new(&hash)
            .map(|h| Argon2::default().verify_password(password.as_bytes(), &h).is_ok())
            .unwrap_or(false)
    })
    .await
}

fn secure(state: &AppState, headers: &HeaderMap) -> bool {
    state.config().trust_proxy
        && headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.eq_ignore_ascii_case("https"))
}

fn set_cookie(value: &str, max_age: i64, secure: bool) -> HeaderValue {
    let s = format!(
        "{COOKIE}={value}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Strict{}",
        if secure { "; Secure" } else { "" }
    );
    HeaderValue::from_str(&s).expect("cookie ASCII")
}

/// 32 octets aléatoires en base64url (jetons de session, clés d'API).
pub(crate) fn random_token() -> String {
    let mut raw = [0u8; 32];
    rand::fill(&mut raw);
    URL_SAFE_NO_PAD.encode(raw)
}

/// Hôte vu par le client : `X-Forwarded-Host` derrière un reverse proxy
/// (`ENGINEX_TRUST_PROXY`), sinon `Host`.
fn public_host<'h>(state: &AppState, h: &'h HeaderMap) -> Option<&'h str> {
    if state.config().trust_proxy {
        h.get("x-forwarded-host").or_else(|| h.get(header::HOST))
    } else {
        h.get(header::HOST)
    }
    .and_then(|v| v.to_str().ok())
}

/// Origine publique d'EngineX (`https://hôte` derrière un proxy en HTTPS).
pub(crate) fn origin(state: &AppState, headers: &HeaderMap) -> String {
    let scheme = if secure(state, headers) { "https" } else { "http" };
    format!("{scheme}://{}", public_host(state, headers).unwrap_or("localhost:8090"))
}

/// Crée une session et renvoie l'en-tête `Set-Cookie`.
async fn new_session(state: &AppState, headers: &HeaderMap) -> Result<HeaderValue, ApiError> {
    let token = random_token();
    let now = now_secs();
    sqlx::query("INSERT INTO sessions (token_hash, created_at, last_seen, expires_at) VALUES (?, ?, ?, ?)")
        .bind(sha256(token.as_bytes()))
        .bind(now)
        .bind(now)
        .bind(now + SESSION_SECS)
        .execute(state.db())
        .await?;
    Ok(set_cookie(&token, SESSION_SECS, secure(state, headers)))
}

fn cookie_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == COOKIE)
        .map(|(_, v)| v.to_string())
}

/// Session valide : prolonge sa durée et renvoie l'empreinte du jeton.
async fn touch_session(state: &AppState, headers: &HeaderMap) -> Result<Option<Vec<u8>>, ApiError> {
    let Some(token) = cookie_token(headers) else {
        return Ok(None);
    };
    let hash = sha256(token.as_bytes());
    let now = now_secs();
    let done = sqlx::query("UPDATE sessions SET last_seen = ?, expires_at = ? WHERE token_hash = ? AND expires_at > ?")
        .bind(now)
        .bind(now + SESSION_SECS)
        .bind(&hash)
        .bind(now)
        .execute(state.db())
        .await?;
    Ok((done.rows_affected() == 1).then_some(hash))
}

pub async fn require_session(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    match touch_session(&state, req.headers()).await {
        Ok(Some(hash)) => {
            req.extensions_mut().insert(SessionId(hash));
            next.run(req).await
        }
        Ok(None) => ApiError::Unauthorized.into_response(),
        Err(e) => e.into_response(),
    }
}

/// Partie `hôte[:port]` d'une origine `schéma://hôte[:port]`.
fn origin_host(origin: &str) -> Option<&str> {
    let rest = origin.split_once("://")?.1;
    let host = rest.split('/').next()?;
    (!host.is_empty()).then_some(host)
}

/// Refuse les requêtes d'écriture venues d'une autre origine.
pub async fn same_origin(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if req.method() == Method::GET || req.method() == Method::HEAD {
        return next.run(req).await;
    }
    let h = req.headers();
    if h.get(header::ORIGIN).is_some() && !origin_ok(&state, h) {
        return ApiError::ForbiddenOrigin.into_response();
    }
    next.run(req).await
}

/// L'en-tête `Origin` est présent et désigne l'hôte public d'EngineX.
pub fn origin_ok(state: &AppState, h: &HeaderMap) -> bool {
    let Some(origin) = h.get(header::ORIGIN) else {
        return false;
    };
    let origin = origin.to_str().unwrap_or("null");
    match (origin_host(origin), public_host(state, h)) {
        (Some(o), Some(e)) => o.eq_ignore_ascii_case(e),
        _ => false,
    }
}

// ---- Gestionnaires ----

pub async fn auth_state(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let setup_required = ensure_setup_code(&state).await?;
    let authenticated = !setup_required && touch_session(&state, &headers).await?.is_some();
    Ok(Json(
        json!({ "setup_required": setup_required, "authenticated": authenticated }),
    ))
}

#[derive(Deserialize)]
pub struct SetupBody {
    code: String,
    password: String,
}

pub async fn setup(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<SetupBody>,
) -> Result<Response, ApiError> {
    let ip = ip_of(&state, &peer, &headers);
    check_limiter(&state, &ip)?;
    if !ensure_setup_code(&state).await? {
        return Err(ApiError::AlreadySetup);
    }
    let expected = state.setup_code().map(|c| normalize_code(&c)).unwrap_or_default();
    // Comparaison des empreintes : tableaux de même taille, quelle que soit
    // la longueur saisie.
    if sha256(normalize_code(&body.code).as_bytes()) != sha256(expected.as_bytes()) {
        record_failure(&state, &ip);
        return Err(ApiError::BadSetupCode);
    }
    if body.password.chars().count() < MIN_PASSWORD {
        return Err(ApiError::PasswordTooShort);
    }
    let hash = hash_password(&state, body.password).await?;
    let done = sqlx::query("INSERT INTO settings (key, value) VALUES ('password_hash', ?) ON CONFLICT(key) DO NOTHING")
        .bind(hash)
        .execute(state.db())
        .await?;
    if done.rows_affected() == 0 {
        return Err(ApiError::AlreadySetup);
    }
    *state.0.setup_code.lock().expect("verrou") = None;
    clear_failures(&state, &ip);
    tracing::info!("mot de passe configuré");
    let cookie = new_session(&state, &headers).await?;
    Ok(([(header::SET_COOKIE, cookie)], Json(json!({ "ok": true }))).into_response())
}

#[derive(Deserialize)]
pub struct LoginBody {
    password: String,
}

pub async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ApiJson(body): ApiJson<LoginBody>,
) -> Result<Response, ApiError> {
    let ip = ip_of(&state, &peer, &headers);
    check_limiter(&state, &ip)?;
    let ok = match password_hash(&state).await? {
        Some(hash) => verify_password(&state, body.password, hash).await?,
        None => false,
    };
    if !ok {
        record_failure(&state, &ip);
        return Err(ApiError::BadCredentials);
    }
    clear_failures(&state, &ip);
    sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
        .bind(now_secs())
        .execute(state.db())
        .await?;
    let cookie = new_session(&state, &headers).await?;
    Ok(([(header::SET_COOKIE, cookie)], Json(json!({ "ok": true }))).into_response())
}

pub async fn logout(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Extension(SessionId(hash)): axum::Extension<SessionId>,
) -> Result<Response, ApiError> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
        .bind(hash)
        .execute(state.db())
        .await?;
    let cookie = set_cookie("", 0, secure(&state, &headers));
    Ok(([(header::SET_COOKIE, cookie)], Json(json!({ "ok": true }))).into_response())
}

#[derive(Deserialize)]
pub struct PasswordBody {
    current: String,
    new: String,
}

pub async fn change_password(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    axum::Extension(SessionId(session)): axum::Extension<SessionId>,
    ApiJson(body): ApiJson<PasswordBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let ip = ip_of(&state, &peer, &headers);
    check_limiter(&state, &ip)?;
    let hash = password_hash(&state).await?.ok_or(ApiError::BadCredentials)?;
    if !verify_password(&state, body.current, hash).await? {
        record_failure(&state, &ip);
        return Err(ApiError::BadCredentials);
    }
    clear_failures(&state, &ip);
    if body.new.chars().count() < MIN_PASSWORD {
        return Err(ApiError::PasswordTooShort);
    }
    let new_hash = hash_password(&state, body.new).await?;
    let mut tx = state.db().begin().await?;
    sqlx::query("UPDATE settings SET value = ? WHERE key = 'password_hash'")
        .bind(new_hash)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE token_hash != ?")
        .bind(session)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    tracing::info!("mot de passe changé, autres sessions fermées");
    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_format() {
        let c = new_code();
        assert_eq!(c.len(), 14);
        assert_eq!(normalize_code(&c).len(), 12);
        assert_eq!(normalize_code(" abcd-efgh-jk mn "), "ABCDEFGHJKMN");
    }

    #[test]
    fn origins() {
        assert_eq!(origin_host("https://enginex:8090"), Some("enginex:8090"));
        assert_eq!(origin_host("http://a.b/x"), Some("a.b"));
        assert_eq!(origin_host("null"), None);
    }

    #[test]
    fn cookie_parse() {
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; enginex_session=tok; b=2"),
        );
        assert_eq!(cookie_token(&h).as_deref(), Some("tok"));
    }
}
