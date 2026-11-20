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

pub async fn require_api_key(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    match authenticate(&state, peer_of(req.extensions()), req.headers()).await {
        Ok((_, internal, name)) => {
            req.extensions_mut().insert(Caller { internal, name });
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

/// Clé de la requête (posée par [`require_api_key`]).
#[derive(Clone)]
pub struct Caller {
    pub internal: bool,
    pub name: String,
}

/// Place dans la file commune (`queue.rs`). Une sandbox dont le tour tient un
/// bail passe ; une sandbox occupée sans bail (approbation tout juste donnée)
/// attend que son bail revienne ; une requête hors tour ou d'une clé externe
/// prend un bail le temps de la requête, sur le modèle chargé.
async fn queue_slot(state: &AppState, caller: Option<&Caller>) -> Result<Option<crate::queue::Lease>, String> {
    use crate::queue::{Ask, USER, Who};
    let Some(c) = caller else { return Ok(None) };
    let label = if c.internal {
        let sid = c.name.strip_prefix("sandbox:").unwrap_or(&c.name);
        let who = Who::Session(sid.to_string());
        let hub = crate::bridge::hub(state, sid);
        loop {
            if state.queue().holds(&who) {
                return Ok(None);
            }
            if !hub.busy() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        format!("session {sid} (hors tour)")
    } else {
        format!("clé d'API « {} »", c.name)
    };
    let ask = Ask {
        who: state.queue().request_id(),
        priority: USER,
        preset: None,
        label,
    };
    state
        .queue()
        .acquire(state, ask, &|_| {}, std::future::pending())
        .await
        .map(Some)
}

// ---- Preset actif ----

/// Réflexion du preset pour la traduction (budget réglé ensuite par requête).
fn think(s: &PresetSettings) -> enginex_oai::Think {
    enginex_oai::Think {
        on: s.reasoning == enginex_engine::args::Reasoning::On,
        budget: s.reasoning_budget,
    }
}

pub(crate) struct Active {
    pub id: i64,
    pub name: String,
    pub settings: PresetSettings,
    pub vision: bool,
    pub updated_at: i64,
}

pub(crate) async fn active(state: &AppState) -> Result<Option<Active>, V1Error> {
    let id = presets::active_id(state).await.map_err(internal_err)?;
    match id {
        Some(id) => preset_info(state, id).await,
        None => Ok(None),
    }
}

fn internal_err(e: crate::error::ApiError) -> V1Error {
    tracing::error!("preset actif : {e}");
    V1Error::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "server_error",
        "lecture du preset actif",
    )
}

/// Réglages et vision du preset `id` (`None` s'il n'existe plus).
pub(crate) async fn preset_info(state: &AppState, id: i64) -> Result<Option<Active>, V1Error> {
    let p = match presets::preset(state, id).await {
        Ok(p) => p,
        Err(crate::error::ApiError::NotFound) => return Ok(None),
        Err(e) => return Err(internal_err(e)),
    };
    let settings: PresetSettings = serde_json::from_str(&p.settings).map_err(|e| internal_err(e.into()))?;
    let m = models::model(state, p.model_id).await.map_err(internal_err)?;
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
    _lease: Option<crate::queue::Lease>,
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

/// Attend la place dans la file en émettant des maintiens ; `Ok(None)` =
/// client parti.
#[allow(clippy::type_complexity)]
async fn wait_slot(
    state: &AppState,
    caller: Option<&Caller>,
    tx: &Tx,
    tr: &mut Translator,
) -> Result<Option<Option<crate::queue::Lease>>, String> {
    let t = &state.config().timings;
    let deadline = Instant::now() + t.wait_limit;
    let fut = queue_slot(state, caller);
    tokio::pin!(fut);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err("file d'attente : le modèle est occupé par d'autres tours depuis trop longtemps".into());
        }
        tokio::select! {
            r = &mut fut => return r.map(Some),
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
    /// Temps processeur de llama-server au moment où la requête lui part.
    cpu0: Option<(u32, f64, Instant)>,
    /// Session de la sandbox appelante (débit par action dans le fil).
    session: Option<String>,
}

/// Secondes de processeur consommées par llama-server (`/proc/<pid>/stat`).
fn llama_cpu(state: &AppState) -> Option<(u32, f64, Instant)> {
    let EngineState::Ready { pid, .. } = state.supervisor().state() else {
        return None;
    };
    let ticks = stat_ticks(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)?;
    // SAFETY: sysconf sans effet de bord.
    let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f64;
    Some((pid, ticks as f64 / hz, Instant::now()))
}

/// utime + stime (champs 14 et 15) d'une ligne `/proc/<pid>/stat` ; le nom
/// entre parenthèses peut contenir des espaces.
fn stat_ticks(stat: &str) -> Option<u64> {
    let rest = stat.get(stat.rfind(')')? + 1..)?;
    let f: Vec<&str> = rest.split_whitespace().collect();
    Some(f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?)
}

/// Cœurs occupés en moyenne par llama-server pendant la requête (si elle a
/// duré assez pour que la mesure ait un sens). Tout sur GPU : ≈ 1 cœur (le
/// fil qui attend les cartes) ; des couches sur le processeur : un cœur par
/// fil de calcul.
fn cpu_cores(state: &AppState, cpu0: Option<(u32, f64, Instant)>) -> Option<f64> {
    let (pid0, t0, at) = cpu0?;
    let (pid1, t1, _) = llama_cpu(state)?;
    let elapsed = at.elapsed().as_secs_f64();
    (pid0 == pid1 && elapsed >= 3.0).then(|| ((t1 - t0) / elapsed * 10.0).round() / 10.0)
}

/// `vitesse=… t/s brouillon=a/n (p %)` tiré des `timings` de llama-server.
fn speed_of(t: Option<&Value>) -> (String, Option<Value>) {
    let Some(t) = t else { return (String::new(), None) };
    let f = |k: &str| t.get(k).and_then(Value::as_f64);
    let tps = f("predicted_per_second").filter(|v| v.is_finite() && *v > 0.0);
    let draft = f("draft_n").filter(|n| *n > 0.0);
    let accepted = f("draft_n_accepted");
    let mut s = String::new();
    if let Some(v) = tps {
        s.push_str(&format!(" vitesse={v:.1} t/s"));
    }
    if let (Some(n), Some(a)) = (draft, accepted) {
        s.push_str(&format!(" brouillon={a}/{n} ({:.0} %)", a * 100.0 / n));
    }
    let stats = tps.map(|v| {
        json!({
            "at": now(),
            "tokens_per_s": (v * 10.0).round() / 10.0,
            "predicted": f("predicted_n"),
            "predicted_ms": f("predicted_ms"),
            "draft_n": draft,
            "draft_accepted": draft.and(accepted),
        })
    });
    (s, stats)
}

impl Log {
    fn write(&self, state: &AppState, outcome: &str, usage: Option<&Value>, timings: Option<&Value>) {
        let tok = |k: &str| usage.and_then(|u| u.get(k)).and_then(Value::as_u64).unwrap_or(0);
        let dropped = if self.dropped.is_empty() {
            String::new()
        } else {
            let d: Vec<String> = self.dropped.iter().map(|(k, n)| format!("{k}×{n}")).collect();
            format!(" ignorés=[{}]", d.join(", "))
        };
        let (mut speed, stats) = speed_of(timings);
        let cores = cpu_cores(state, self.cpu0);
        if let Some(c) = cores {
            speed.push_str(&format!(" cpu={c} cœurs"));
        }
        if let Some(mut st) = stats {
            st["preset"] = json!(self.preset);
            st["cpu_cores"] = json!(cores);
            if let Some(sid) = &self.session {
                crate::bridge::hub(state, sid).note_gen(&st);
            }
            *state.0.last_gen.lock().expect("stats") = Some(st);
        }
        tracing::info!(
            "{} preset=« {} » [{}] {} ms entrée={} sortie={}{} → {}{}",
            self.route,
            self.preset,
            self.sampling,
            self.started.elapsed().as_millis(),
            tok("prompt_tokens"),
            tok("completion_tokens"),
            speed,
            outcome,
            dropped
        );
    }
}

pub async fn responses(
    State(state): State<AppState>,
    caller: Option<axum::Extension<Caller>>,
    body: Bytes,
) -> Response {
    let caller = caller.map(|c| c.0);
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
        think: p.as_ref().map(|p| think(&p.settings)).unwrap_or_default(),
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
        cpu0: llama_cpu(&state),
        session: caller.as_ref().and_then(|c| {
            c.name
                .strip_prefix("sandbox:")
                .filter(|_| c.internal)
                .map(str::to_string)
        }),
    };
    tokio::spawn(async move {
        let mut outcome = None;
        let ok = relay_responses(&state, caller.as_ref(), &tx, &mut tr, &body, &mut log, &mut outcome).await;
        let usage = tr.usage().cloned();
        let result = match (ok, outcome) {
            (false, _) => "client parti".to_string(),
            (true, Some(code)) => code,
            (true, None) => tr.finish_reason().unwrap_or("fin").to_string(),
        };
        log.write(&state, &result, usage.as_ref(), tr.timings());
    });
    sse_response(rx)
}

/// `false` si le client est parti.
async fn relay_responses(
    state: &AppState,
    caller: Option<&Caller>,
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
    let lease = match wait_slot(state, caller, tx, tr).await {
        Ok(Some(l)) => l,
        Ok(None) => {
            state.mark_done();
            return false;
        }
        Err(msg) => {
            state.mark_done();
            return send(tx, tr.fail("server_error", &msg), outcome).await;
        }
    };
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
        _lease: lease,
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
                think: think(&p.settings),
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

pub async fn chat_completions(
    State(state): State<AppState>,
    caller: Option<axum::Extension<Caller>>,
    body: Bytes,
) -> Response {
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
    let limit = state.config().timings.wait_limit;
    let lease = match tokio::time::timeout(limit, queue_slot(&state, caller.as_ref().map(|c| &c.0))).await {
        Ok(Ok(l)) => l,
        Ok(Err(msg)) => {
            state.mark_done();
            return V1Error::new(StatusCode::SERVICE_UNAVAILABLE, "engine_unavailable", msg).into_response();
        }
        Err(_) => {
            state.mark_done();
            return V1Error::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "engine_unavailable",
                "file d'attente : le modèle est occupé par d'autres tours depuis trop longtemps",
            )
            .into_response();
        }
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
        _lease: lease,
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
        cpu0: llama_cpu(&state),
        session: None,
    };
    let streamed = req.get("stream").and_then(Value::as_bool) == Some(true);
    let res = match state.0.http.post(chat_url(&state)).json(&req).send().await {
        Ok(r) => r,
        Err(e) => {
            tracing::warn!("llama-server injoignable : {e}");
            log.write(&state, "server_error", None, None);
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
        let body = serde_json::from_slice::<Value>(&bytes).ok();
        let field = |k: &str| body.as_ref().and_then(|v| v.get(k));
        log.write(
            &state,
            &format!("HTTP {}", status.as_u16()),
            field("usage"),
            field("timings"),
        );
        drop(served);
        return (status, [(header::CONTENT_TYPE, ctype)], bytes).into_response();
    }
    let (tx, rx) = mpsc::channel::<Result<Bytes, Infallible>>(64);
    tokio::spawn(async move {
        let _served = served;
        let mut stream = res.bytes_stream();
        let mut outcome = "fin";
        // Débit et jetons : lus au passage dans les derniers événements SSE.
        let mut pending = Vec::<u8>::new();
        let (mut usage, mut timings) = (None::<Value>, None::<Value>);
        loop {
            let next = tokio::select! {
                n = stream.next() => n,
                _ = tx.closed() => { outcome = "client parti"; break }
            };
            match next {
                Some(Ok(b)) => {
                    pending.extend_from_slice(&b);
                    while let Some(i) = pending.iter().position(|c| *c == b'\n') {
                        let line: Vec<u8> = pending.drain(..=i).collect();
                        let Some(data) = line.strip_prefix(b"data: ") else {
                            continue;
                        };
                        if !(data.windows(9).any(|w| w == b"\"timings\"") || data.windows(7).any(|w| w == b"\"usage\""))
                        {
                            continue;
                        }
                        if let Ok(v) = serde_json::from_slice::<Value>(data) {
                            if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
                                usage = Some(u.clone());
                            }
                            if let Some(t) = v.get("timings").filter(|t| t.is_object()) {
                                timings = Some(t.clone());
                            }
                        }
                    }
                    if pending.len() > 1 << 20 {
                        pending.clear();
                    }
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
        log.write(&state, outcome, usage.as_ref(), timings.as_ref());
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
            "context_window": crate::engine_api::effective_ctx(&state, p.id, p.settings.ctx),
        }]}))
        .into_response(),
        Ok(None) => Json(json!({"object": "list", "data": []})).into_response(),
        Err(e) => e.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_stat_ticks() {
        let line = "4242 (llama server) S 1 4242 4242 0 -1 4194560 1 0 0 0 1500 250 0 0 20 0 9 0 100";
        assert_eq!(stat_ticks(line), Some(1750));
        assert_eq!(stat_ticks("4242 (x) S 1"), None);
        let own = std::fs::read_to_string("/proc/self/stat").unwrap();
        assert!(stat_ticks(&own).is_some());
    }
}
