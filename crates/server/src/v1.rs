//! API compatible OpenAI pour Codex : `/v1/responses` (traduit vers le
//! chat/completions de llama-server par le crate `oai`),
//! `/v1/chat/completions` (relayé tel quel), `/v1/models`.
//!
//! Protégée par une clé d'API (pas de cookie de session, pas de garde « même
//! origine ») ; erreurs au format OpenAI (`V1Error`). Chaque requête prend la
//! porte de service en lecture jusqu'à la fin de son flux : un chargement de
//! llama-server ne la coupe jamais, et elle ne part que si le moteur est prêt.

use std::{collections::BTreeMap, convert::Infallible, net::SocketAddr, time::Instant};

use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{ConnectInfo, DefaultBodyLimit, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use enginex_engine::{args::PresetSettings, supervisor::EngineState};
use enginex_oai::{Ctx, Event, Translator, random_id, sampling};
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::sync::{OwnedRwLockReadGuard, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use crate::{
    api_keys,
    auth::ip_of,
    error::V1Error,
    models, presets,
    state::{AppState, now},
};

/// Corps maximal de `/v1` (une image en data: URL et un long contexte).
pub const BODY_LIMIT: usize = 32 * 1024 * 1024;

pub fn router(state: &AppState) -> Router<AppState> {
    Router::new()
        .route("/v1/responses", post(responses))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/models", get(list_models))
        .route_layer(middleware::from_fn_with_state(state.clone(), require_api_key))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
}

/// Masque les secrets dans chaque chaîne du corps JSON (les valeurs
/// échappées sont comparées décodées). Corps illisible : rendu tel quel, la
/// traduction le refusera.
fn mask_body(state: &AppState, body: Bytes) -> Bytes {
    let m = state.masker();
    if m.is_empty() {
        return body;
    }
    match serde_json::from_slice::<Value>(&body) {
        Ok(mut v) => {
            m.mask_json(&mut v);
            Bytes::from(serde_json::to_vec(&v).expect("JSON"))
        }
        Err(_) => body,
    }
}

/// Routes permises aux clés `internal` (sandboxes).
const INTERNAL_ROUTES: &[&str] = &["/v1/responses", "/v1/models"];

// ---- Clé d'API ----

pub(crate) fn bearer(headers: &HeaderMap) -> Option<&str> {
    let v = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let (scheme, key) = v.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then(|| key.trim())
}

/// Contrôle de la clé d'API (limiteur d'échecs par IP compris) : rend
/// `(id, internal, name)` de la clé, ou la réponse d'erreur au format `/v1`.
pub(crate) async fn authenticate(
    state: &AppState,
    peer: SocketAddr,
    headers: &HeaderMap,
) -> Result<(i64, bool, String), V1Error> {
    let ip = ip_of(state, &peer, headers);
    let wait = state
        .0
        .v1_limiter
        .lock()
        .expect("verrou")
        .locked_for(&ip, Instant::now());
    if !wait.is_zero() {
        return Err(V1Error::new(
            StatusCode::TOO_MANY_REQUESTS,
            "too_many_failures",
            format!(
                "trop de clés refusées depuis cette adresse : réessayer dans {} s",
                wait.as_secs().max(1)
            ),
        ));
    }
    let found = match bearer(headers) {
        Some(k) if k.starts_with(api_keys::PREFIX) => api_keys::find(state, k).await,
        _ => Ok(None),
    };
    match found {
        Ok(Some(k)) => {
            state.0.v1_limiter.lock().expect("verrou").clear(&ip);
            Ok(k)
        }
        Ok(None) => {
            state
                .0
                .v1_limiter
                .lock()
                .expect("verrou")
                .record_failure(&ip, Instant::now());
            Err(V1Error::new(
                StatusCode::UNAUTHORIZED,
                "invalid_api_key",
                "clé d'API absente ou inconnue (Paramètres → API)",
            ))
        }
        Err(e) => {
            tracing::error!("clé d'API : {e}");
            Err(V1Error::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "base de données",
            ))
        }
    }
}

pub(crate) fn peer_of(ext: &axum::http::Extensions) -> SocketAddr {
    ext.get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0)
        .unwrap_or_else(|| SocketAddr::from(([0, 0, 0, 0], 0)))
}

pub async fn require_api_key(State(state): State<AppState>, req: Request, next: Next) -> Response {
    match authenticate(&state, peer_of(req.extensions()), req.headers()).await {
        Ok((_, internal, _)) => {
            if internal && !INTERNAL_ROUTES.contains(&req.uri().path()) {
                return V1Error::new(
                    StatusCode::FORBIDDEN,
                    "key_scope",
                    "clé de sandbox : seules /v1/responses et /v1/models sont permises",
                )
                .into_response();
            }
            next.run(req).await
        }
        Err(e) => e.into_response(),
    }
}

// ---- Preset actif ----

pub(crate) struct Active {
    pub id: i64,
    pub name: String,
    pub settings: PresetSettings,
    pub vision: bool,
    pub updated_at: i64,
}

pub(crate) async fn active(state: &AppState) -> Result<Option<Active>, V1Error> {
    let internal = |e: crate::error::ApiError| {
        tracing::error!("preset actif : {e}");
        V1Error::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "lecture du preset actif",
        )
    };
    let Some(id) = presets::active_id(state).await.map_err(internal)? else {
        return Ok(None);
    };
    let p = match presets::preset(state, id).await {
        Ok(p) => p,
        Err(crate::error::ApiError::NotFound) => return Ok(None),
        Err(e) => return Err(internal(e)),
    };
    let settings: PresetSettings = serde_json::from_str(&p.settings).map_err(|e| internal(e.into()))?;
    let m = models::model(state, p.model_id).await.map_err(internal)?;
    let vision = settings.mmproj && m.mmproj.is_some();
    Ok(Some(Active {
        id,
        updated_at: p.updated_at,
        name: p.name,
        settings,
        vision,
    }))
}

fn unavailable() -> V1Error {
    V1Error::new(
        StatusCode::SERVICE_UNAVAILABLE,
        "engine_unavailable",
        "Aucun modèle chargé : activer un preset dans Modèles",
    )
}

/// Requête servie : porte en lecture tenue, `last_done` posé à la fin (y
/// compris quand le client coupe la connexion).
struct Served {
    state: AppState,
    _gate: Option<OwnedRwLockReadGuard<()>>,
}

impl Drop for Served {
    fn drop(&mut self) {
        self.state.mark_done();
    }
}

pub(crate) fn not_ready(st: &EngineState) -> Option<String> {
    match st {
        EngineState::Ready { .. } => None,
        EngineState::Failed { cause, .. } => Some(format!("le moteur a échoué : {cause}")),
        EngineState::Stopped => Some("le moteur est arrêté".into()),
        _ => Some("le moteur n'est pas prêt".into()),
    }
}

fn chat_url(state: &AppState) -> String {
    format!("http://127.0.0.1:{}/v1/chat/completions", state.supervisor().port())
}

// ---- /v1/responses ----

type Tx = mpsc::Sender<Result<Bytes, Infallible>>;

async fn send(tx: &Tx, events: Vec<Event>, outcome: &mut Option<String>) -> bool {
    for e in events {
        if e.name == "response.failed" {
            *outcome = e
                .data
                .pointer("/response/error/code")
                .and_then(Value::as_str)
                .map(str::to_string);
        }
        if tx.send(Ok(Bytes::from(e.to_sse()))).await.is_err() {
            return false;
        }
    }
    true
}

fn sse_response(rx: mpsc::Receiver<Result<Bytes, Infallible>>) -> Response {
    Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(ReceiverStream::new(rx)))
        .expect("réponse SSE")
}

/// Attend la porte en lecture en émettant un maintien toutes les
/// `keepalive` ; `Err` = délai dépassé, `Ok(None)` = client parti.
async fn wait_gate(state: &AppState, tx: &Tx, tr: &mut Translator) -> Result<Option<OwnedRwLockReadGuard<()>>, ()> {
    let t = &state.config().timings;
    let deadline = Instant::now() + t.wait_limit;
    let acquire = state.serve_gate().clone().read_owned();
    tokio::pin!(acquire);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(());
        }
        tokio::select! {
            g = &mut acquire => return Ok(Some(g)),
            _ = tx.closed() => return Ok(None),
            _ = tokio::time::sleep(t.keepalive.min(left)) => {
                if Instant::now() < deadline
                    && tx.send(Ok(Bytes::from(tr.keepalive().to_sse()))).await.is_err()
                {
                    return Ok(None);
                }
            }
        }
    }
}

struct Log {
    route: &'static str,
    preset: String,
    sampling: String,
    started: Instant,
    dropped: BTreeMap<String, usize>,
}

impl Log {
    fn write(&self, outcome: &str, usage: Option<&Value>) {
        let tok = |k: &str| usage.and_then(|u| u.get(k)).and_then(Value::as_u64).unwrap_or(0);
        let dropped = if self.dropped.is_empty() {
            String::new()
        } else {
            let d: Vec<String> = self.dropped.iter().map(|(k, n)| format!("{k}×{n}")).collect();
            format!(" ignorés=[{}]", d.join(", "))
        };
        tracing::info!(
            "{} preset=« {} » [{}] {} ms entrée={} sortie={} → {}{}",
            self.route,
            self.preset,
            self.sampling,
            self.started.elapsed().as_millis(),
            tok("prompt_tokens"),
            tok("completion_tokens"),
            outcome,
            dropped
        );
    }
}

pub async fn responses(State(state): State<AppState>, body: Bytes) -> Response {
    let started = Instant::now();
    let body = mask_body(&state, body);
    let preset = match active(&state).await {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };
    let ctx = |p: &Option<Active>| Ctx {
        model_name: p.as_ref().map(|p| p.name.clone()).unwrap_or_default(),
        sampling: p.as_ref().map(|p| p.settings.sampling.clone()).unwrap_or_default(),
        // Sans preset la requête finit en 503 : ne pas refuser ses images avant.
        vision: p.as_ref().is_none_or(|p| p.vision),
    };
    let translated = match enginex_oai::parse_and_translate(&body, &ctx(&preset)) {
        Ok(t) => t,
        Err(e) => {
            tracing::info!("/v1/responses refusée : {} ({e})", e.code());
            return V1Error::new(StatusCode::BAD_REQUEST, e.code(), e.to_string()).into_response();
        }
    };
    let Some(p) = preset else {
        return unavailable().into_response();
    };
    if !translated.dropped.is_empty() {
        tracing::warn!("/v1/responses : éléments ou outils ignorés {:?}", translated.dropped);
    }
    let (tx, rx) = mpsc::channel(64);
    let mut tr = Translator::new(random_id("resp"), p.name.clone(), translated.names, Box::new(random_id))
        .with_created_at(now().max(0) as u64);
    let mut log = Log {
        route: "/v1/responses",
        preset: p.name,
        sampling: String::new(),
        started,
        dropped: translated.dropped,
    };
    tokio::spawn(async move {
        let mut outcome = None;
        let ok = relay_responses(&state, &tx, &mut tr, &body, &mut log, &mut outcome).await;
        let usage = tr.usage().cloned();
        let result = match (ok, outcome) {
            (false, _) => "client parti".to_string(),
            (true, Some(code)) => code,
            (true, None) => tr.finish_reason().unwrap_or("fin").to_string(),
        };
        log.write(&result, usage.as_ref());
    });
    sse_response(rx)
}

/// `false` si le client est parti.
async fn relay_responses(
    state: &AppState,
    tx: &Tx,
    tr: &mut Translator,
    raw: &Bytes,
    log: &mut Log,
    outcome: &mut Option<String>,
) -> bool {
    if !send(tx, tr.start(), outcome).await {
        state.mark_done();
        return false;
    }
    let gate = match wait_gate(state, tx, tr).await {
        Ok(Some(g)) => g,
        Ok(None) => {
            state.mark_done();
            return false;
        }
        Err(()) => {
            state.mark_done();
            let msg = "le moteur n'est pas devenu disponible à temps (chargement trop long)";
            return send(tx, tr.fail("server_error", msg), outcome).await;
        }
    };
    let _served = Served {
        state: state.clone(),
        _gate: Some(gate),
    };
    if let Some(cause) = not_ready(&state.supervisor().state()) {
        return send(tx, tr.fail("server_error", &cause), outcome).await;
    }
    // Le preset a pu changer pendant l'attente : traduction refaite avec
    // l'actuel (l'échantillonnage modifié s'applique tout de suite).
    let body = match active(state).await {
        Ok(Some(p)) => {
            let ctx = Ctx {
                model_name: p.name.clone(),
                sampling: p.settings.sampling.clone(),
                vision: p.vision,
            };
            match enginex_oai::parse_and_translate(raw, &ctx) {
                Ok(t) => t.body,
                Err(e) => return send(tx, tr.fail("invalid_request_error", &e.to_string()), outcome).await,
            }
        }
        Ok(None) => return send(tx, tr.fail("server_error", "aucun preset actif"), outcome).await,
        Err(e) => return send(tx, tr.fail("server_error", &e.message), outcome).await,
    };
    log.sampling = sampling::describe(&body);
    let res = match state.0.http.post(chat_url(state)).json(&body).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("llama-server injoignable : {e}");
            return send(tx, tr.fail("server_error", "llama-server injoignable"), outcome).await;
        }
    };
    let status = res.status();
    if !status.is_success() {
        let text = res.text().await.unwrap_or_default();
        let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        let msg = v
            .pointer("/error/message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("llama-server : HTTP {status}"));
        let code = if v.pointer("/error/type").and_then(Value::as_str) == Some("exceed_context_size_error") {
            "context_length_exceeded"
        } else if status.is_client_error() {
            "invalid_request_error"
        } else {
            "server_error"
        };
        tracing::warn!("llama-server HTTP {status} : {msg}");
        return send(tx, tr.fail(code, &msg), outcome).await;
    }
    let keepalive = state.config().timings.keepalive;
    let mut stream = res.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    let mut done = false;
    'read: loop {
        let next = tokio::select! {
            n = tokio::time::timeout(keepalive, stream.next()) => n,
            _ = tx.closed() => return false,
        };
        let chunk = match next {
            Err(_) => {
                if tx.send(Ok(Bytes::from(tr.keepalive().to_sse()))).await.is_err() {
                    return false;
                }
                continue;
            }
            Ok(None) => break,
            Ok(Some(Err(e))) => {
                tracing::warn!("flux de llama-server coupé : {e}");
                break;
            }
            Ok(Some(Ok(c))) => c,
        };
        buf.extend_from_slice(&chunk);
        while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&line);
            let Some(data) = line.trim_end().strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data == "[DONE]" {
                done = true;
                break 'read;
            }
            match serde_json::from_str::<Value>(data) {
                Ok(v) => {
                    if !send(tx, tr.push_chunk(&v), outcome).await {
                        return false;
                    }
                    if tr.is_closed() {
                        return true;
                    }
                }
                Err(e) => tracing::warn!("morceau illisible de llama-server : {e}"),
            }
        }
    }
    send(tx, tr.finish(done), outcome).await
}

// ---- /v1/chat/completions ----

/// Attend la porte (sans événements) puis vérifie que le moteur est prêt.
async fn gate_for_chat(state: &AppState) -> Result<OwnedRwLockReadGuard<()>, V1Error> {
    let limit = state.config().timings.wait_limit;
    let g = tokio::time::timeout(limit, state.serve_gate().clone().read_owned())
        .await
        .map_err(|_| {
            V1Error::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "engine_unavailable",
                "le moteur n'est pas devenu disponible",
            )
        })?;
    if let Some(cause) = not_ready(&state.supervisor().state()) {
        return Err(V1Error::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "engine_unavailable",
            cause,
        ));
    }
    Ok(g)
}

pub async fn chat_completions(State(state): State<AppState>, body: Bytes) -> Response {
    let started = Instant::now();
    let mut req: Value = match serde_json::from_slice(&body) {
        Ok(mut v @ Value::Object(_)) => {
            state.masker().mask_json(&mut v);
            v
        }
        _ => {
            return V1Error::new(StatusCode::BAD_REQUEST, "invalid_request_error", "JSON illisible").into_response();
        }
    };
    let p = match active(&state).await {
        Ok(Some(p)) => p,
        Ok(None) => return unavailable().into_response(),
        Err(e) => return e.into_response(),
    };
    let gate = match gate_for_chat(&state).await {
        Ok(g) => g,
        Err(e) => {
            state.mark_done();
            return e.into_response();
        }
    };
    let served = Served {
        state: state.clone(),
        _gate: Some(gate),
    };
    // Preset relu après l'attente (il a pu changer).
    let p = match active(&state).await {
        Ok(Some(p)) => p,
        _ => p,
    };
    req["model"] = json!(p.name);
    sampling::inject(&mut req, &p.settings.sampling);
    let log = Log {
        route: "/v1/chat/completions",
        preset: p.name.clone(),
        sampling: sampling::describe(&req),
        started,
        dropped: BTreeMap::new(),
    };
    let streamed = req.get("stream").and_then(Value::as_bool) == Some(true);
    let res = match state.0.http.post(chat_url(&state)).json(&req).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("llama-server injoignable : {e}");
            log.write("server_error", None);
            return V1Error::new(StatusCode::BAD_GATEWAY, "server_error", "llama-server injoignable").into_response();
        }
    };
    let status = StatusCode::from_u16(res.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let ctype = res
        .headers()
        .get(header::CONTENT_TYPE)
        .cloned()
        .unwrap_or_else(|| header::HeaderValue::from_static("application/json"));
    if !streamed || !status.is_success() {
        let bytes = res.bytes().await.unwrap_or_default();
        let usage = serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|v| v.get("usage").cloned());
        log.write(&format!("HTTP {}", status.as_u16()), usage.as_ref());
        drop(served);
        return (status, [(header::CONTENT_TYPE, ctype)], bytes).into_response();
    }
    let (tx, rx) = mpsc::channel::<Result<Bytes, Infallible>>(64);
    tokio::spawn(async move {
        let _served = served;
        let mut stream = res.bytes_stream();
        let mut outcome = "fin";
        loop {
            let next = tokio::select! {
                n = stream.next() => n,
                _ = tx.closed() => { outcome = "client parti"; break }
            };
            match next {
                Some(Ok(b)) => {
                    if tx.send(Ok(b)).await.is_err() {
                        outcome = "client parti";
                        break;
                    }
                }
                Some(Err(e)) => {
                    tracing::warn!("flux de llama-server coupé : {e}");
                    outcome = "flux coupé";
                    break;
                }
                None => break,
            }
        }
        log.write(outcome, None);
    });
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, ctype)
        .header(header::CACHE_CONTROL, "no-cache")
        .body(Body::from_stream(ReceiverStream::new(rx)))
        .expect("réponse")
}

// ---- /v1/models ----

pub async fn list_models(State(state): State<AppState>) -> Response {
    match active(&state).await {
        Ok(Some(p)) => Json(json!({"object": "list", "data": [{
            "id": p.name, "object": "model", "owned_by": "enginex", "created": 0,
            "context_window": p.settings.ctx,
        }]}))
        .into_response(),
        Ok(None) => Json(json!({"object": "list", "data": []})).into_response(),
        Err(e) => e.into_response(),
    }
}
