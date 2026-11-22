//! Aperçu des apps des sessions, sur une origine à part (second port).
//!
//! Le code de l'app (généré, paquets npm) ne tourne jamais sur l'origine de
//! l'UI : il ne lit ni l'API ni le coffre. L'UI demande un laissez-passer
//! (`POST /api/sessions/{id}/preview`) ; `/__enginex/open?g=…` sur le port
//! d'aperçu le pose en cookie puis sert l'app à la racine (chemins absolus et
//! HMR de Vite compris). Un aperçu actif par navigateur : le dernier ouvert.

use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    body::Body,
    extract::{Path as UrlPath, Query, Request, State},
    http::{HeaderMap, HeaderValue, Response, StatusCode, header},
    response::IntoResponse,
};
use hyper_util::rt::TokioIo;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    auth, bridge,
    error::{ApiError, ApiJson},
    sessions, snapshots,
    state::AppState,
};

pub const COOKIE: &str = "enginex_preview";
/// « Port » du laissez-passer qui sert les fichiers du dossier de la session
/// (pages HTML sans serveur) au lieu d'une app de la sandbox.
pub const FILES: u16 = 0;
/// Taille maximale d'un fichier servi en mode fichiers.
const FILES_MAX: usize = 32 << 20;
/// Laissez-passer gardés au plus (les plus vieux partent d'abord).
const MAX_GRANTS: usize = 256;
const GRANT_TTL: Duration = Duration::from_secs(7 * 24 * 3600);

struct Grant {
    session_id: String,
    port: u16,
    created: Instant,
}

#[derive(Default)]
pub struct Grants(Mutex<HashMap<String, Grant>>);

impl Grants {
    fn create(&self, session_id: &str, port: u16) -> String {
        let token = auth::random_token();
        let mut g = self.0.lock().expect("verrou");
        g.retain(|_, v| v.created.elapsed() < GRANT_TTL);
        while g.len() >= MAX_GRANTS {
            let oldest = g.iter().min_by_key(|(_, v)| v.created).map(|(k, _)| k.clone());
            match oldest {
                Some(k) => g.remove(&k),
                None => break,
            };
        }
        g.insert(
            token.clone(),
            Grant {
                session_id: session_id.into(),
                port,
                created: Instant::now(),
            },
        );
        token
    }

    fn get(&self, token: &str) -> Option<(String, u16)> {
        let g = self.0.lock().expect("verrou");
        g.get(token)
            .filter(|v| v.created.elapsed() < GRANT_TTL)
            .map(|v| (v.session_id.clone(), v.port))
    }

    /// Retire les laissez-passer d'une session supprimée.
    pub fn forget_session(&self, session_id: &str) {
        self.0.lock().expect("verrou").retain(|_, v| v.session_id != session_id);
    }
}

/// Origine du port d'aperçu vue par le navigateur : `ENGINEX_PREVIEW_URL`,
/// sinon le schéma et l'hôte de l'UI avec le port d'aperçu publié.
pub fn origin(state: &AppState, headers: &HeaderMap) -> Option<String> {
    let w = &state.config().web;
    w.preview_bind?;
    if let Some(u) = &w.preview_url {
        return Some(u.trim_end_matches('/').to_string());
    }
    let ui = auth::origin(state, headers);
    let (scheme, rest) = ui.split_once("://")?;
    // `hôte:port`, `[ipv6]:port` ou `hôte` seul.
    let host = match rest.rfind(':') {
        Some(i) if !rest[i..].contains(']') => &rest[..i],
        _ => rest,
    };
    Some(format!("{scheme}://{host}:{}", w.preview_public_port))
}

// ---- API de l'UI (port principal, session exigée) ----

pub async fn status(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let row = sessions::row(&state, &id).await?;
    let base = origin(&state, &headers);
    // Page à servir sans app : `index.html` à la racine du dossier de la session.
    let has_index = match snapshots::folder(&state, &row).await {
        Ok(f) => tokio::task::spawn_blocking(move || crate::files::kind_beneath(&f.work, "index.html") == Some(false))
            .await
            .unwrap_or(false),
        Err(_) => false,
    };
    let Some((fwd, token)) = bridge::hub(&state, &id).preview_target() else {
        return Ok(Json(
            json!({"running": false, "ports": [], "enabled": base.is_some(), "has_index": has_index}),
        ));
    };
    let (ports, error) = match enginex_sandbox::fwd::list(fwd.as_deref(), &token).await {
        Ok(p) => (p, None),
        Err(e) => (Vec::new(), Some(e.to_string())),
    };
    Ok(Json(
        json!({"running": true, "ports": ports, "error": error, "enabled": base.is_some(), "has_index": has_index}),
    ))
}

#[derive(Deserialize)]
pub struct OpenBody {
    port: u16,
}

pub async fn open(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
    ApiJson(b): ApiJson<OpenBody>,
) -> Result<Json<Value>, ApiError> {
    sessions::row(&state, &id).await?;
    let base =
        origin(&state, &headers).ok_or(ApiError::Invalid("aperçu désactivé (ENGINEX_PREVIEW_BIND=off)".into()))?;
    // `FILES` (0) : les fichiers du dossier, pas une app.
    let g = state.0.previews.create(&id, b.port);
    Ok(Json(
        json!({"url": format!("{base}/__enginex/open?g={g}"), "origin": base}),
    ))
}

// ---- Port d'aperçu ----

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/__enginex/open", axum::routing::get(enter))
        .fallback(proxy)
        .with_state(state)
}

#[derive(Deserialize)]
struct EnterQuery {
    g: String,
}

pub fn page(status: StatusCode, msg: &str) -> Response<Body> {
    let html = format!(
        "<!doctype html><meta charset=utf-8><title>Aperçu EngineX</title>\
         <body style=\"font:15px system-ui;margin:3em;color:#333\"><p>{msg}</p></body>"
    );
    let mut r = (status, html).into_response();
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    r.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

async fn enter(State(state): State<AppState>, Query(q): Query<EnterQuery>) -> Response<Body> {
    if state.0.previews.get(&q.g).is_none() {
        return page(
            StatusCode::FORBIDDEN,
            "Lien d'aperçu inconnu ou expiré : rouvre l'aperçu depuis la session.",
        );
    }
    let mut r = Response::new(Body::empty());
    *r.status_mut() = StatusCode::FOUND;
    let h = r.headers_mut();
    h.insert(header::LOCATION, HeaderValue::from_static("/"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Ok(v) = HeaderValue::from_str(&format!("{COOKIE}={}; Path=/; HttpOnly; SameSite=Lax", q.g)) {
        h.insert(header::SET_COOKIE, v);
    }
    r
}

fn cookie_pairs(h: &HeaderMap) -> impl Iterator<Item = (String, String)> + '_ {
    h.get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|p| {
            let (k, v) = p.trim().split_once('=')?;
            Some((k.to_string(), v.to_string()))
        })
}

/// Cookies envoyés à l'app : jamais ceux d'EngineX (la session de l'UI part
/// aussi vers ce port, les cookies ignorant le port).
pub fn app_cookies(h: &HeaderMap) -> Option<String> {
    let kept: Vec<String> = cookie_pairs(h)
        .filter(|(k, _)| !k.starts_with("enginex_"))
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    (!kept.is_empty()).then(|| kept.join("; "))
}

/// `Set-Cookie` de l'app qui viserait un cookie d'EngineX.
pub fn enginex_set_cookie(v: &HeaderValue) -> bool {
    v.to_str()
        .ok()
        .and_then(|s| s.split('=').next())
        .is_some_and(|k| k.trim().starts_with("enginex_"))
}

const HOP: [&str; 6] = [
    "keep-alive",
    "proxy-connection",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
];

/// En-têtes de la requête relayée : `Host` de l'app (Vite refuse un autre
/// hôte), cookies filtrés, en-têtes de saut retirés (`Connection`/`Upgrade`
/// gardés pour un WebSocket).
pub fn upstream_headers(h: &HeaderMap, port: u16, upgrade: bool) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (k, v) in h {
        let n = k.as_str();
        if HOP.contains(&n) || n == "host" || n == "cookie" {
            continue;
        }
        if !upgrade && (n == "connection" || n == "upgrade") {
            continue;
        }
        out.append(k.clone(), v.clone());
    }
    if let Ok(v) = HeaderValue::from_str(&format!("localhost:{port}")) {
        out.insert(header::HOST, v);
    }
    if let Some(c) = app_cookies(h).and_then(|c| HeaderValue::from_str(&c).ok()) {
        out.insert(header::COOKIE, c);
    }
    out
}

async fn proxy(State(state): State<AppState>, req: Request) -> Response<Body> {
    // Toutes les valeurs : l'app peut poser son propre `enginex_preview` sur un
    // chemin plus long, envoyé en premier par le navigateur.
    let grant = cookie_pairs(req.headers())
        .filter(|(k, _)| k == COOKIE)
        .take(4)
        .find_map(|(_, v)| state.0.previews.get(&v));
    let Some((sid, port)) = grant else {
        return page(
            StatusCode::FORBIDDEN,
            "Pas d'aperçu ouvert dans ce navigateur : ouvre-le depuis l'onglet Aperçu d'une session.",
        );
    };
    if port == FILES {
        let path = req.uri().path().to_string();
        return serve_files(&state, &sid, &path).await;
    }
    forward(&state, &sid, port, req).await
}

/// `%XX` décodés (chemin d'URL) ; `None` si le résultat n'est pas de l'UTF-8.
fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(v) = std::str::from_utf8(&b[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn files_type(rel: &str) -> &'static str {
    let ext = rel
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" | "cjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "wasm" => "application/wasm",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "txt" | "md" | "csv" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// Sert un fichier du dossier de la session (mode `FILES`), sur l'origine
/// d'aperçu : la page peut lancer son JavaScript sans toucher à l'UI. Lecture
/// par `files::read_beneath` (aucun lien suivi) ; pas besoin de la sandbox.
async fn serve_files(state: &AppState, sid: &str, raw: &str) -> Response<Body> {
    let Ok(row) = sessions::row(state, sid).await else {
        return page(StatusCode::NOT_FOUND, "Session supprimée.");
    };
    let Ok(folder) = snapshots::folder(state, &row).await else {
        return page(StatusCode::NOT_FOUND, "Dossier de la session introuvable.");
    };
    let Some(path) = percent_decode(raw) else {
        return page(StatusCode::BAD_REQUEST, "Chemin illisible.");
    };
    let rel = path.trim_matches('/').to_string();
    if !rel.is_empty() && !snapshots::valid_rel(&rel) {
        return page(StatusCode::NOT_FOUND, "Fichier introuvable.");
    }
    let work = folder.work;
    let dir_slash = path.ends_with('/');
    let res = tokio::task::spawn_blocking(move || {
        let is_dir = rel.is_empty() || crate::files::kind_beneath(&work, &rel) == Some(true);
        if is_dir && !rel.is_empty() && !dir_slash {
            // Dossier sans « / » final : les liens relatifs de sa page casseraient.
            return Err(Some(format!("/{rel}/")));
        }
        let file = if is_dir {
            if rel.is_empty() {
                "index.html".to_string()
            } else {
                format!("{rel}/index.html")
            }
        } else {
            rel
        };
        match crate::files::read_beneath(&work, &file, FILES_MAX) {
            Some(b) if b.len() <= FILES_MAX => Ok((file, b)),
            _ => Err(None),
        }
    })
    .await;
    match res {
        Ok(Ok((file, bytes))) => {
            let mut r = Response::new(Body::from(bytes));
            let h = r.headers_mut();
            h.insert(header::CONTENT_TYPE, HeaderValue::from_static(files_type(&file)));
            h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
            h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            r
        }
        Ok(Err(Some(to))) => {
            let mut r = Response::new(Body::empty());
            *r.status_mut() = StatusCode::FOUND;
            if let Ok(v) = HeaderValue::from_str(&to) {
                r.headers_mut().insert(header::LOCATION, v);
            }
            r
        }
        _ => page(
            StatusCode::NOT_FOUND,
            "Fichier introuvable dans le dossier de la session.",
        ),
    }
}

/// Relaie une requête (HTTP ou WebSocket) vers l'app de la sandbox de `sid`
/// qui écoute sur `port` : `Host: localhost:<port>`, cookies `enginex_*`
/// retirés dans les deux sens. Sert le port d'aperçu et les sas du
/// navigateur piloté (`appgate.rs`).
pub async fn forward(state: &AppState, sid: &str, port: u16, mut req: Request) -> Response<Body> {
    let hub = bridge::hub(state, sid);
    let Some((fwd, token)) = hub.preview_target() else {
        return page(
            StatusCode::SERVICE_UNAVAILABLE,
            "La sandbox de cette session est arrêtée : envoie un message dans la session pour la relancer, puis démarre l'app.",
        );
    };
    hub.touch_activity();
    let stream = match enginex_sandbox::fwd::connect(fwd.as_deref(), &token, port).await {
        Ok(s) => s,
        Err(e) => {
            return page(
                StatusCode::BAD_GATEWAY,
                &format!("L'app ne répond pas sur le port {port} : {e}. Lance-la dans la session (npm run dev…)."),
            );
        }
    };
    let upgrade = req
        .headers()
        .get(header::UPGRADE)
        .is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"websocket"));
    let (mut sender, conn) = match hyper::client::conn::http1::handshake(TokioIo::new(stream)).await {
        Ok(x) => x,
        Err(e) => return page(StatusCode::BAD_GATEWAY, &format!("connexion à l'app : {e}")),
    };
    tokio::spawn(async move {
        let _ = conn.with_upgrades().await;
    });
    let path = req
        .uri()
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| "/".into());
    let incoming_upgrade = upgrade.then(|| hyper::upgrade::on(&mut req));
    let (parts, body) = req.into_parts();
    let mut up = hyper::Request::builder().method(parts.method).uri(path);
    if let Some(h) = up.headers_mut() {
        *h = upstream_headers(&parts.headers, port, upgrade);
    }
    let Ok(up) = up.body(body) else {
        return page(StatusCode::BAD_REQUEST, "requête invalide");
    };
    let mut res = match sender.send_request(up).await {
        Ok(r) => r,
        Err(e) => return page(StatusCode::BAD_GATEWAY, &format!("réponse de l'app : {e}")),
    };
    if res.status() == StatusCode::SWITCHING_PROTOCOLS
        && let Some(down) = incoming_upgrade
    {
        let up_upgrade = hyper::upgrade::on(&mut res);
        tokio::spawn(async move {
            if let (Ok(a), Ok(b)) = (down.await, up_upgrade.await) {
                let _ = tokio::io::copy_bidirectional(&mut TokioIo::new(a), &mut TokioIo::new(b)).await;
            }
        });
    }
    let (mut parts, body) = res.into_parts();
    let cookies: Vec<HeaderValue> = parts
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .filter(|v| !enginex_set_cookie(v))
        .cloned()
        .collect();
    parts.headers.remove(header::SET_COOKIE);
    for c in cookies {
        parts.headers.append(header::SET_COOKIE, c);
    }
    if parts.status != StatusCode::SWITCHING_PROTOCOLS {
        for h in HOP {
            parts.headers.remove(h);
        }
    }
    Response::from_parts(parts, Body::new(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookies_of_enginex_never_reach_the_app() {
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            HeaderValue::from_static("enginex_session=SECRET; theme=dark; enginex_preview=G; sid=1"),
        );
        assert_eq!(app_cookies(&h).as_deref(), Some("theme=dark; sid=1"));
        let up = upstream_headers(&h, 5173, false);
        assert_eq!(up[header::HOST], "localhost:5173");
        assert_eq!(up[header::COOKIE], "theme=dark; sid=1");
        h.insert(header::COOKIE, HeaderValue::from_static("enginex_session=SECRET"));
        assert!(upstream_headers(&h, 5173, false).get(header::COOKIE).is_none());
    }

    #[test]
    fn upgrade_headers_kept_only_for_websocket() {
        let mut h = HeaderMap::new();
        h.insert(header::CONNECTION, HeaderValue::from_static("Upgrade"));
        h.insert(header::UPGRADE, HeaderValue::from_static("websocket"));
        h.insert("keep-alive", HeaderValue::from_static("5"));
        let ws = upstream_headers(&h, 1, true);
        assert!(ws.get(header::UPGRADE).is_some() && ws.get("keep-alive").is_none());
        let plain = upstream_headers(&h, 1, false);
        assert!(plain.get(header::UPGRADE).is_none() && plain.get(header::CONNECTION).is_none());
    }

    #[test]
    fn set_cookie_filter() {
        assert!(enginex_set_cookie(&HeaderValue::from_static(
            "enginex_session=x; Path=/"
        )));
        assert!(!enginex_set_cookie(&HeaderValue::from_static("sid=x; Path=/")));
    }

    #[test]
    fn grants_are_capped() {
        let g = Grants::default();
        let first = g.create("s", 1);
        for _ in 0..MAX_GRANTS {
            g.create("s", 2);
        }
        assert!(g.get(&first).is_none());
        let t = g.create("a", 3);
        g.forget_session("a");
        assert!(g.get(&t).is_none());
    }
}
