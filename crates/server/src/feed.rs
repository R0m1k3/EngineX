//! Fil de notifications (étape 7b) : ce qui s'est passé pendant que Michael
//! ne regardait pas (cron fini, tour de fond fini, vérification rouge,
//! approbation attendue, téléchargement, moteur, disque, webhooks entrants),
//! en direct dans l'UI (SSE) et en push web (VAPID, RFC 8291).

use std::{
    collections::HashMap,
    convert::Infallible,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use axum::{
    Json,
    body::{Body, Bytes},
    extract::{Path as UrlPath, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::wrappers::ReceiverStream;
use web_push_native::{Auth, WebPushBuilder, jwt_simple::algorithms::ES256KeyPair, p256::PublicKey};

use crate::{
    auth,
    error::{ApiError, ApiJson},
    state::{AppState, now},
    system::{set_setting, setting},
};

/// Entrées gardées.
pub const KEEP: i64 = 500;
const TITLE_MAX: usize = 200;
const BODY_MAX: usize = 2000;
/// Corps d'un webhook entrant.
pub const HOOK_BODY_MAX: usize = 64 * 1024;
/// Appels par minute et par webhook.
const HOOK_RATE: u32 = 30;
/// Abonnements push gardés.
const SUBS_MAX: i64 = 20;
/// Une alerte disque au plus toutes les 12 h.
const DISK_EVERY: Duration = Duration::from_secs(12 * 3600);

pub const LEVELS: [&str; 4] = ["info", "success", "warning", "error"];

/// Une notification à poster.
#[derive(Clone, Debug)]
pub struct Note {
    pub source: &'static str,
    pub level: &'static str,
    pub title: String,
    pub body: String,
    pub link: Option<String>,
    /// Part en push en mode « important ».
    pub important: bool,
}

impl Note {
    pub fn new(source: &'static str, level: &'static str, title: impl Into<String>) -> Self {
        Self {
            source,
            level,
            title: title.into(),
            body: String::new(),
            link: None,
            important: level == "error",
        }
    }
    pub fn body(mut self, b: impl Into<String>) -> Self {
        self.body = b.into();
        self
    }
    pub fn link(mut self, l: impl Into<String>) -> Self {
        self.link = Some(l.into());
        self
    }
    pub fn important(mut self, on: bool) -> Self {
        self.important = on;
        self
    }
}

#[derive(Default)]
pub struct Feed {
    tx: OnceLock<broadcast::Sender<Value>>,
    vapid: OnceLock<ES256KeyPair>,
    hook_hits: Mutex<HashMap<i64, (Instant, u32)>>,
    disk_last: Mutex<Option<Instant>>,
}

impl Feed {
    fn tx(&self) -> &broadcast::Sender<Value> {
        self.tx.get_or_init(|| broadcast::channel(256).0)
    }
}

fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

/// Poste en fond (n'attend pas la base ni le push).
pub fn emit(state: &AppState, note: Note) {
    let s = state.clone();
    tokio::spawn(async move {
        if let Err(e) = post(&s, note).await {
            tracing::warn!("fil de notifications : {e}");
        }
    });
}

/// Enregistre, diffuse et pousse une notification ; rend son id.
pub async fn post(state: &AppState, note: Note) -> Result<i64, sqlx::Error> {
    let masker = state.masker();
    let title = cut(&masker.mask(note.title.trim()), TITLE_MAX);
    let body = cut(&masker.mask(note.body.trim()), BODY_MAX);
    let at = now();
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO feed (at, source, level, title, body, link) VALUES (?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(at)
    .bind(note.source)
    .bind(note.level)
    .bind(&title)
    .bind(&body)
    .bind(&note.link)
    .fetch_one(state.db())
    .await?;
    sqlx::query("DELETE FROM feed WHERE id <= ?")
        .bind(id - KEEP)
        .execute(state.db())
        .await?;
    let item = json!({"id": id, "at": at, "source": note.source, "level": note.level, "title": title,
        "body": body, "link": note.link, "read": false});
    let unread = unread(state).await.unwrap_or(0);
    let _ = state.0.feed.tx().send(json!({"item": item, "unread": unread}));
    let mode = push_mode(state).await;
    if mode == "all" || (mode == "important" && note.important) {
        let s = state.clone();
        let payload = json!({"title": title, "body": body, "link": note.link, "level": note.level, "id": id});
        tokio::spawn(async move { send_push(&s, &payload).await });
    }
    Ok(id)
}

async fn unread(state: &AppState) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM feed WHERE read = 0")
        .fetch_one(state.db())
        .await
}

// ---- API du fil ----

#[derive(Deserialize)]
pub struct ListQuery {
    before: Option<i64>,
    limit: Option<i64>,
    unread: Option<bool>,
}

type Row = (i64, i64, String, String, String, String, Option<String>, bool);

fn row_json(r: Row) -> Value {
    json!({"id": r.0, "at": r.1, "source": r.2, "level": r.3, "title": r.4, "body": r.5, "link": r.6, "read": r.7})
}

pub async fn list(State(state): State<AppState>, Query(q): Query<ListQuery>) -> Result<Json<Value>, ApiError> {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, at, source, level, title, body, link, read FROM feed WHERE id < ? AND (? = 0 OR read = 0) \
         ORDER BY id DESC LIMIT ?",
    )
    .bind(q.before.unwrap_or(i64::MAX))
    .bind(q.unread.unwrap_or(false) as i64)
    .bind(q.limit.unwrap_or(50).clamp(1, 200))
    .fetch_all(state.db())
    .await?;
    Ok(Json(json!({
        "items": rows.into_iter().map(row_json).collect::<Vec<_>>(),
        "unread": unread(&state).await?,
    })))
}

#[derive(Deserialize)]
pub struct ReadBody {
    #[serde(default)]
    ids: Vec<i64>,
    #[serde(default)]
    all: bool,
}

pub async fn mark_read(State(state): State<AppState>, ApiJson(b): ApiJson<ReadBody>) -> Result<Json<Value>, ApiError> {
    if b.all {
        sqlx::query("UPDATE feed SET read = 1 WHERE read = 0")
            .execute(state.db())
            .await?;
    }
    for id in b.ids.iter().take(500) {
        sqlx::query("UPDATE feed SET read = 1 WHERE id = ?")
            .bind(id)
            .execute(state.db())
            .await?;
    }
    let n = unread(&state).await?;
    let _ = state.0.feed.tx().send(json!({"unread": n}));
    Ok(Json(json!({"unread": n})))
}

pub async fn delete_one(State(state): State<AppState>, UrlPath(id): UrlPath<i64>) -> Result<Json<Value>, ApiError> {
    sqlx::query("DELETE FROM feed WHERE id = ?")
        .bind(id)
        .execute(state.db())
        .await?;
    let n = unread(&state).await?;
    let _ = state.0.feed.tx().send(json!({"unread": n}));
    Ok(Json(json!({"unread": n})))
}

pub async fn clear(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    sqlx::query("DELETE FROM feed").execute(state.db()).await?;
    let _ = state.0.feed.tx().send(json!({"unread": 0}));
    Ok(Json(json!({"unread": 0})))
}

fn sse(name: &str, data: &Value) -> Bytes {
    Bytes::from(format!("event: {name}\ndata: {data}\n\n"))
}

/// SSE : `unread` à l'ouverture, puis `feed` à chaque changement.
pub async fn events(State(state): State<AppState>) -> Result<Response, ApiError> {
    let mut rx = state.0.feed.tx().subscribe();
    let first = unread(&state).await?;
    let (tx, out) = mpsc::channel::<Result<Bytes, Infallible>>(64);
    let keepalive = state.config().timings.keepalive.max(Duration::from_millis(100));
    tokio::spawn(async move {
        if tx.send(Ok(sse("feed", &json!({"unread": first})))).await.is_err() {
            return;
        }
        loop {
            tokio::select! {
                ev = rx.recv() => match ev {
                    Ok(v) => if tx.send(Ok(sse("feed", &v))).await.is_err() { return },
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let n = unread(&state).await.unwrap_or(0);
                        if tx.send(Ok(sse("feed", &json!({"unread": n, "reload": true})))).await.is_err() { return }
                    }
                    Err(_) => return,
                },
                _ = tokio::time::sleep(keepalive) => {
                    if tx.send(Ok(Bytes::from_static(b": ping\n\n"))).await.is_err() { return }
                }
                _ = tx.closed() => return,
            }
        }
    });
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("x-accel-buffering", "no")
        .body(Body::from_stream(ReceiverStream::new(out)))
        .expect("réponse SSE"))
}

// ---- Push web ----

async fn push_mode(state: &AppState) -> String {
    setting(state, "push.mode")
        .await
        .ok()
        .flatten()
        .filter(|m| matches!(m.as_str(), "important" | "all" | "off"))
        .unwrap_or_else(|| "important".into())
}

/// Clé VAPID du serveur (`<data>/vapid.key`, créée au premier besoin).
fn vapid(state: &AppState) -> Result<&ES256KeyPair, String> {
    if let Some(k) = state.0.feed.vapid.get() {
        return Ok(k);
    }
    let path = state.config().data_dir.join("vapid.key");
    let kp = match std::fs::read(&path) {
        Ok(raw) => ES256KeyPair::from_bytes(&raw).map_err(|e| format!("vapid.key illisible : {e}"))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let kp = ES256KeyPair::generate();
            crate::sandboxes::write_private(&path, &kp.to_bytes()).map_err(|e| format!("vapid.key : {e}"))?;
            kp
        }
        Err(e) => return Err(format!("vapid.key : {e}")),
    };
    Ok(state.0.feed.vapid.get_or_init(|| kp))
}

/// Clé publique VAPID, point non compressé (`applicationServerKey`).
pub fn vapid_public(state: &AppState) -> Result<String, String> {
    use web_push_native::p256::elliptic_curve::sec1::ToEncodedPoint;
    let compressed = vapid(state)?.public_key().to_bytes();
    let pk = PublicKey::from_sec1_bytes(&compressed).map_err(|e| format!("clé VAPID : {e}"))?;
    Ok(URL_SAFE_NO_PAD.encode(pk.to_encoded_point(false).as_bytes()))
}

pub async fn push_status(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let key = vapid_public(&state).map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))?;
    let subs: Vec<(i64, String, String, i64, Option<i64>)> =
        sqlx::query_as("SELECT id, endpoint, agent, created_at, last_ok FROM push_subs ORDER BY id")
            .fetch_all(state.db())
            .await?;
    Ok(Json(json!({
        "public_key": key,
        "mode": push_mode(&state).await,
        "subscriptions": subs.into_iter().map(|(id, ep, agent, at, ok)| json!({
            "id": id, "endpoint": ep, "host": reqwest::Url::parse(&ep).ok().and_then(|u| u.host_str().map(str::to_string)),
            "agent": agent, "created_at": at, "last_ok": ok})).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
pub struct SubKeys {
    p256dh: String,
    auth: String,
}

#[derive(Deserialize)]
pub struct SubBody {
    endpoint: String,
    keys: SubKeys,
}

/// Adresse d'un service de push : https (http seulement vers la machine
/// elle-même, pour les tests).
fn check_endpoint(ep: &str) -> Result<(), ApiError> {
    let u = reqwest::Url::parse(ep).map_err(|_| ApiError::Invalid("adresse de push illisible".into()))?;
    let local = matches!(u.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    if ep.len() > 2048 || !(u.scheme() == "https" || (u.scheme() == "http" && local)) {
        return Err(ApiError::Invalid("adresse de push : https attendu".into()));
    }
    Ok(())
}

pub async fn subscribe(
    State(state): State<AppState>,
    headers: HeaderMap,
    ApiJson(b): ApiJson<SubBody>,
) -> Result<Json<Value>, ApiError> {
    check_endpoint(&b.endpoint)?;
    let p = URL_SAFE_NO_PAD
        .decode(b.keys.p256dh.trim_end_matches('='))
        .ok()
        .filter(|k| PublicKey::from_sec1_bytes(k).is_ok());
    let a = URL_SAFE_NO_PAD
        .decode(b.keys.auth.trim_end_matches('='))
        .ok()
        .filter(|a| a.len() == 16);
    if p.is_none() || a.is_none() {
        return Err(ApiError::Invalid("clés d'abonnement illisibles".into()));
    }
    let agent: String = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .chars()
        .take(200)
        .collect();
    sqlx::query(
        "INSERT INTO push_subs (endpoint, p256dh, auth, agent, created_at) VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT(endpoint) DO UPDATE SET p256dh = excluded.p256dh, auth = excluded.auth, agent = excluded.agent",
    )
    .bind(&b.endpoint)
    .bind(&b.keys.p256dh)
    .bind(&b.keys.auth)
    .bind(agent)
    .bind(now())
    .execute(state.db())
    .await?;
    // Les plus anciens partent au-delà de la limite.
    sqlx::query("DELETE FROM push_subs WHERE id NOT IN (SELECT id FROM push_subs ORDER BY id DESC LIMIT ?)")
        .bind(SUBS_MAX)
        .execute(state.db())
        .await?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
pub struct UnsubBody {
    endpoint: Option<String>,
    id: Option<i64>,
}

pub async fn unsubscribe(
    State(state): State<AppState>,
    ApiJson(b): ApiJson<UnsubBody>,
) -> Result<Json<Value>, ApiError> {
    sqlx::query("DELETE FROM push_subs WHERE endpoint = ? OR id = ?")
        .bind(b.endpoint)
        .bind(b.id)
        .execute(state.db())
        .await?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
pub struct ModeBody {
    mode: String,
}

pub async fn set_mode(State(state): State<AppState>, ApiJson(b): ApiJson<ModeBody>) -> Result<Json<Value>, ApiError> {
    if !matches!(b.mode.as_str(), "important" | "all" | "off") {
        return Err(ApiError::BadRequest);
    }
    set_setting(&state, "push.mode", Some(&b.mode)).await?;
    Ok(Json(json!({"mode": b.mode})))
}

/// Envoie une notification d'essai à tous les navigateurs abonnés.
pub async fn push_test(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let r = send_push(
        &state,
        &json!({"title": "EngineX", "body": "Notification d'essai : le push fonctionne.", "level": "info"}),
    )
    .await;
    Ok(Json(json!({"sent": r.0, "failed": r.1, "errors": r.2})))
}

/// Pousse `payload` à chaque abonnement ; (envoyés, échecs, erreurs).
async fn send_push(state: &AppState, payload: &Value) -> (usize, usize, Vec<String>) {
    let kp = match vapid(state) {
        Ok(k) => k,
        Err(e) => {
            tracing::warn!("push : {e}");
            return (0, 0, vec![e]);
        }
    };
    let subs: Vec<(i64, String, String, String)> =
        match sqlx::query_as("SELECT id, endpoint, p256dh, auth FROM push_subs")
            .fetch_all(state.db())
            .await
        {
            Ok(s) => s,
            Err(e) => return (0, 0, vec![e.to_string()]),
        };
    let body = payload.to_string();
    let (mut ok, mut ko, mut errors) = (0, 0, Vec::new());
    for (id, ep, p256dh, a) in subs {
        let r = async {
            let ua = URL_SAFE_NO_PAD
                .decode(p256dh.trim_end_matches('='))
                .map_err(|e| e.to_string())?;
            let auth = URL_SAFE_NO_PAD
                .decode(a.trim_end_matches('='))
                .map_err(|e| e.to_string())?;
            let key = PublicKey::from_sec1_bytes(&ua).map_err(|e| e.to_string())?;
            let req = WebPushBuilder::new(
                ep.parse().map_err(|e: axum::http::uri::InvalidUri| e.to_string())?,
                key,
                Auth::clone_from_slice(&auth),
            )
            .with_valid_duration(Duration::from_secs(24 * 3600))
            .with_vapid(kp, "mailto:enginex@localhost")
            .build(body.clone())
            .map_err(|e| e.to_string())?;
            let (parts, data) = req.into_parts();
            let mut rb = state.0.http.post(&ep).body(data).timeout(Duration::from_secs(15));
            for (k, v) in parts.headers.iter() {
                rb = rb.header(k.as_str(), v.as_bytes());
            }
            let res = rb.send().await.map_err(|e| e.to_string())?;
            Ok::<u16, String>(res.status().as_u16())
        }
        .await;
        match r {
            Ok(s) if (200..300).contains(&s) => {
                ok += 1;
                let _ = sqlx::query("UPDATE push_subs SET last_ok = ? WHERE id = ?")
                    .bind(now())
                    .bind(id)
                    .execute(state.db())
                    .await;
            }
            Ok(s) if s == 404 || s == 410 => {
                // Abonnement expiré côté navigateur.
                ko += 1;
                let _ = sqlx::query("DELETE FROM push_subs WHERE id = ?")
                    .bind(id)
                    .execute(state.db())
                    .await;
                errors.push(format!("abonnement {id} expiré ({s}), retiré"));
            }
            Ok(s) => {
                ko += 1;
                errors.push(format!("abonnement {id} : HTTP {s}"));
            }
            Err(e) => {
                ko += 1;
                errors.push(format!("abonnement {id} : {e}"));
            }
        }
    }
    if !errors.is_empty() {
        tracing::warn!("push : {}", errors.join(" ; "));
    }
    (ok, ko, errors)
}

// ---- Webhooks entrants ----

#[derive(Deserialize)]
pub struct HookBody {
    name: String,
    #[serde(default = "info")]
    level: String,
    #[serde(default)]
    push: bool,
}

fn info() -> String {
    "info".into()
}

fn check_hook(b: &HookBody) -> Result<(), ApiError> {
    let n = b.name.trim();
    if n.is_empty() || n.chars().count() > 80 || !LEVELS.contains(&b.level.as_str()) {
        return Err(ApiError::BadRequest);
    }
    Ok(())
}

/// id, nom, niveau, push, actif, créé, dernier appel, appels.
type HookRow = (i64, String, String, bool, bool, i64, Option<i64>, i64);

pub async fn hooks_list(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let rows: Vec<HookRow> = sqlx::query_as(
        "SELECT id, name, level, push, enabled, created_at, last_at, count FROM webhooks ORDER BY name COLLATE NOCASE",
    )
    .fetch_all(state.db())
    .await?;
    Ok(Json(Value::Array(
        rows.into_iter()
            .map(|r| {
                json!({"id": r.0, "name": r.1, "level": r.2, "push": r.3, "enabled": r.4,
                "created_at": r.5, "last_at": r.6, "count": r.7})
            })
            .collect(),
    )))
}

/// Crée un webhook ; l'adresse (avec son jeton) n'est montrée qu'une fois.
pub async fn hooks_create(
    State(state): State<AppState>,
    ApiJson(b): ApiJson<HookBody>,
) -> Result<Json<Value>, ApiError> {
    check_hook(&b)?;
    let token = auth::random_token();
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO webhooks (name, hash, level, push, created_at) VALUES (?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(b.name.trim())
    .bind(auth::sha256(token.as_bytes()))
    .bind(&b.level)
    .bind(b.push)
    .bind(now())
    .fetch_one(state.db())
    .await?;
    Ok(Json(json!({"id": id, "path": format!("/hooks/{token}")})))
}

#[derive(Deserialize)]
pub struct HookPatch {
    name: Option<String>,
    level: Option<String>,
    push: Option<bool>,
    enabled: Option<bool>,
}

pub async fn hooks_patch(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<i64>,
    ApiJson(b): ApiJson<HookPatch>,
) -> Result<Json<Value>, ApiError> {
    let cur: Option<(String, String, bool, bool)> =
        sqlx::query_as("SELECT name, level, push, enabled FROM webhooks WHERE id = ?")
            .bind(id)
            .fetch_optional(state.db())
            .await?;
    let (name, level, push, enabled) = cur.ok_or(ApiError::NotFound)?;
    let n = HookBody {
        name: b.name.unwrap_or(name),
        level: b.level.unwrap_or(level),
        push: b.push.unwrap_or(push),
    };
    check_hook(&n)?;
    sqlx::query("UPDATE webhooks SET name = ?, level = ?, push = ?, enabled = ? WHERE id = ?")
        .bind(n.name.trim())
        .bind(&n.level)
        .bind(n.push)
        .bind(b.enabled.unwrap_or(enabled))
        .bind(id)
        .execute(state.db())
        .await?;
    Ok(Json(json!({"ok": true})))
}

pub async fn hooks_delete(State(state): State<AppState>, UrlPath(id): UrlPath<i64>) -> Result<Json<Value>, ApiError> {
    let r = sqlx::query("DELETE FROM webhooks WHERE id = ?")
        .bind(id)
        .execute(state.db())
        .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(Json(json!({"ok": true})))
}

fn hook_error(status: StatusCode, msg: &str) -> Response {
    (status, Json(json!({"error": msg}))).into_response()
}

/// `POST /hooks/<jeton>` : sans session ni clé, le jeton de l'adresse suffit.
pub async fn hook_in(
    State(state): State<AppState>,
    UrlPath(token): UrlPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let hook: Option<(i64, String, String, bool, bool)> =
        match sqlx::query_as("SELECT id, name, level, push, enabled FROM webhooks WHERE hash = ?")
            .bind(auth::sha256(token.as_bytes()))
            .fetch_optional(state.db())
            .await
        {
            Ok(h) => h,
            Err(e) => {
                tracing::error!("webhook : {e}");
                return hook_error(StatusCode::INTERNAL_SERVER_ERROR, "erreur interne");
            }
        };
    let Some((id, name, level, push, enabled)) = hook else {
        return hook_error(StatusCode::NOT_FOUND, "webhook inconnu");
    };
    if !enabled {
        return hook_error(StatusCode::FORBIDDEN, "webhook désactivé");
    }
    {
        let mut hits = state.0.feed.hook_hits.lock().expect("webhooks");
        let e = hits.entry(id).or_insert((Instant::now(), 0));
        if e.0.elapsed() >= Duration::from_secs(60) {
            *e = (Instant::now(), 0);
        }
        e.1 += 1;
        if e.1 > HOOK_RATE {
            return hook_error(StatusCode::TOO_MANY_REQUESTS, "trop d'appels (30 par minute)");
        }
    }
    let (title, text) = summarize(&headers, &body);
    let level = LEVELS.iter().find(|l| **l == level).copied().unwrap_or("info");
    let note = Note::new("webhook", level, format!("{name} : {title}"))
        .body(text)
        .important(push);
    let _ = sqlx::query("UPDATE webhooks SET last_at = ?, count = count + 1 WHERE id = ?")
        .bind(now())
        .bind(id)
        .execute(state.db())
        .await;
    match post(&state, note).await {
        Ok(fid) => (StatusCode::ACCEPTED, Json(json!({"ok": true, "id": fid}))).into_response(),
        Err(e) => {
            tracing::error!("webhook : {e}");
            hook_error(StatusCode::INTERNAL_SERVER_ERROR, "erreur interne")
        }
    }
}

fn s<'a>(v: &'a Value, p: &str) -> &'a str {
    v.pointer(p).and_then(Value::as_str).unwrap_or("")
}

/// Titre et texte lisibles d'un appel entrant (GitHub, Gitea, JSON libre, texte).
pub fn summarize(headers: &HeaderMap, body: &[u8]) -> (String, String) {
    let event = ["x-github-event", "x-gitea-event", "x-gogs-event"]
        .iter()
        .find_map(|h| headers.get(*h).and_then(|v| v.to_str().ok()))
        .map(str::to_string);
    let Ok(v) = serde_json::from_slice::<Value>(body) else {
        let text = String::from_utf8_lossy(body).trim().to_string();
        let first = text.lines().next().unwrap_or("").to_string();
        return (if first.is_empty() { "appel reçu".into() } else { first }, text);
    };
    let repo = s(&v, "/repository/full_name");
    if let Some(ev) = event.as_deref() {
        let (t, b) = match ev {
            "push" => {
                let commits = v["commits"].as_array().cloned().unwrap_or_default();
                let branch = s(&v, "/ref").trim_start_matches("refs/heads/").to_string();
                let lines: Vec<String> = commits
                    .iter()
                    .take(10)
                    .map(|c| format!("- {}", s(c, "/message").lines().next().unwrap_or("")))
                    .collect();
                (
                    format!("push sur {repo} ({branch}) : {} commit(s)", commits.len()),
                    lines.join("\n"),
                )
            }
            "pull_request" => (
                format!(
                    "PR #{} {} sur {repo} : {}",
                    v["number"],
                    s(&v, "/action"),
                    s(&v, "/pull_request/title")
                ),
                s(&v, "/pull_request/html_url").to_string(),
            ),
            "issues" => (
                format!(
                    "issue #{} {} sur {repo} : {}",
                    v.pointer("/issue/number").cloned().unwrap_or(Value::Null),
                    s(&v, "/action"),
                    s(&v, "/issue/title")
                ),
                s(&v, "/issue/html_url").to_string(),
            ),
            "workflow_run" => (
                format!(
                    "{} sur {repo} : {}",
                    s(&v, "/workflow_run/name"),
                    match s(&v, "/workflow_run/conclusion") {
                        "" => s(&v, "/workflow_run/status"),
                        c => c,
                    }
                ),
                s(&v, "/workflow_run/html_url").to_string(),
            ),
            "release" => (
                format!("version {} {} sur {repo}", s(&v, "/release/tag_name"), s(&v, "/action")),
                s(&v, "/release/html_url").to_string(),
            ),
            "ping" => ("ping (webhook branché)".into(), String::new()),
            other => (format!("{other} sur {repo}"), String::new()),
        };
        return (t, b);
    }
    let title = ["/title", "/subject", "/event", "/name"]
        .iter()
        .map(|p| s(&v, p))
        .find(|t| !t.is_empty())
        .unwrap_or("appel reçu")
        .to_string();
    let text = ["/text", "/message", "/body", "/content", "/description"]
        .iter()
        .map(|p| s(&v, p))
        .find(|t| !t.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            let pretty = serde_json::to_string_pretty(&v).unwrap_or_default();
            cut(&pretty, 600)
        });
    (title, text)
}

// ---- Surveillance ----

/// Alerte disque (toutes les 10 min, une alerte au plus toutes les 12 h).
pub fn spawn(state: &AppState) {
    let s = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(600)).await;
            check_disk(&s).await;
        }
    });
}

pub async fn check_disk(state: &AppState) {
    let Ok(v) = crate::wsfiles::usage_value(state).await else {
        return;
    };
    if v["alert"] != true {
        return;
    }
    {
        let mut last = state.0.feed.disk_last.lock().expect("disque");
        if last.is_some_and(|t| t.elapsed() < DISK_EVERY) {
            return;
        }
        *last = Some(Instant::now());
    }
    emit(
        state,
        Note::new(
            "disk",
            "warning",
            format!("Disque de /data rempli à {} %", v["used_pct"]),
        )
        .body(format!(
            "Il reste {} Go. Supprimez des sessions, des modèles ou des fichiers.",
            v["free_bytes"].as_u64().unwrap_or(0) / 1_000_000_000
        ))
        .link("#files")
        .important(true),
    );
}

// ---- Émetteurs ----

/// Un tour vient de finir. Les crons postent leur propre résultat ; les
/// tours de mémoire ne disent rien ; les tours de Michael seulement s'ils ont
/// duré (il n'est sans doute plus devant l'écran).
pub fn turn_done(state: &AppState, sid: &str, origin: &str, status: &str, started: i64, error: Option<&str>) {
    const LONG: i64 = 120;
    if matches!(origin, "cron" | "memory") || status == "interrupted" {
        return;
    }
    if matches!(origin, "user" | "verify") && now() - started < LONG {
        return;
    }
    let (s, sid, status, error) = (
        state.clone(),
        sid.to_string(),
        status.to_string(),
        error.map(str::to_string),
    );
    tokio::spawn(async move {
        let title = crate::sessions::row(&s, &sid)
            .await
            .map(|r| r.title)
            .unwrap_or_else(|_| "session".into());
        let note = if status == "failed" {
            Note::new("session", "error", format!("« {title} » : tour en échec")).body(error.unwrap_or_default())
        } else {
            Note::new("session", "success", format!("« {title} » : réponse prête"))
        };
        if let Err(e) = post(&s, note.link(format!("#sessions/{sid}")).important(true)).await {
            tracing::warn!("fil : {e}");
        }
    });
}

/// Codex attend une réponse de Michael (approbation, question).
pub fn waiting_answer(state: &AppState, sid: &str, method: &str) {
    let (s, sid) = (state.clone(), sid.to_string());
    let what = if method == "item/tool/requestUserInput" {
        "pose une question"
    } else {
        "attend votre accord"
    };
    tokio::spawn(async move {
        let title = crate::sessions::row(&s, &sid)
            .await
            .map(|r| r.title)
            .unwrap_or_else(|_| "session".into());
        let note = Note::new("approval", "warning", format!("« {title} » {what}"))
            .link(format!("#sessions/{sid}"))
            .important(true);
        if let Err(e) = post(&s, note).await {
            tracing::warn!("fil : {e}");
        }
    });
}
