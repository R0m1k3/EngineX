//! Navigateur de la stack côté EngineX : outils MCP `browser_*` (un onglet
//! par session) et fenêtre « Regarder » (screencast, souris et clavier de
//! Michael, prise de main).

use axum::{
    Json,
    extract::{
        Path as UrlPath, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::HeaderMap,
    response::Response,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Map, Value, json};

use crate::{auth, error::ApiError, sessions, state::AppState};

/// Schémas des outils (listés seulement si le navigateur est configuré).
pub fn tools() -> Vec<Value> {
    let t = |name: &str, desc: &str, props: Value, req: &[&str]| {
        json!({"name": name, "description": desc, "inputSchema": {
            "type": "object", "properties": props, "required": req, "additionalProperties": false}})
    };
    let reff = json!({"type": "integer", "description": "numéro [n] de l'élément dans le dernier snapshot"});
    vec![
        t(
            "browser_open",
            "Ouvre une page dans le navigateur de la session (onglet gardé entre les appels, cookies et connexions gardés). Rend le snapshot : texte et éléments numérotés [n]. Pour une app de la sandbox, utilise plutôt Playwright ou curl dans la sandbox.",
            json!({"url": {"type": "string", "description": "adresse, ou back / forward"}}),
            &["url"],
        ),
        t(
            "browser_snapshot",
            "Relit la page courante : texte et éléments numérotés [n].",
            json!({}),
            &[],
        ),
        t(
            "browser_find",
            "Cherche un texte ou un élément dans la page (y compris hors de l'écran).",
            json!({"text": {"type": "string"}}),
            &["text"],
        ),
        t(
            "browser_click",
            "Clique l'élément numéroté [n].",
            json!({"ref": reff}),
            &["ref"],
        ),
        t(
            "browser_click_xy",
            "Clique aux coordonnées (pixels de browser_screenshot), pour un élément sans numéro.",
            json!({"x": {"type": "number"}, "y": {"type": "number"}}),
            &["x", "y"],
        ),
        t(
            "browser_type",
            "Saisit du texte dans l'élément [n] (ou là où est le curseur) ; submit=true appuie ensuite sur Entrée.",
            json!({"ref": reff, "text": {"type": "string"}, "submit": {"type": "boolean"}}),
            &["text"],
        ),
        t(
            "browser_key",
            "Appuie sur une touche : Enter, Tab, Escape, ArrowDown, Backspace…",
            json!({"key": {"type": "string"}}),
            &["key"],
        ),
        t(
            "browser_scroll",
            "Fait défiler la page.",
            json!({"direction": {"type": "string", "enum": ["down", "up", "top", "bottom"]}}),
            &["direction"],
        ),
        t(
            "browser_screenshot",
            "Capture la page (image quadrillée tous les 100 px) : pour voir la mise en page ou viser avec browser_click_xy.",
            json!({}),
            &[],
        ),
    ]
}

/// Un outil `browser_*` en contenu MCP (`None` pour un autre nom).
pub async fn call(state: &AppState, session_id: &str, name: &str, args: &Value) -> Option<(Vec<Value>, bool)> {
    let empty = Map::new();
    let args = args.as_object().unwrap_or(&empty);
    let vision = crate::v1::active(state).await.ok().flatten().is_some_and(|a| a.vision);
    let out = state.0.browser.exec(session_id, name, args, vision).await?;
    // Le texte des pages peut contenir un secret du coffre affiché par un site.
    let text = state.masker().mask(&out.result).into_owned();
    let is_error = text.starts_with("[erreur]");
    let mut content = vec![json!({"type": "text", "text": text})];
    if let Some(png) = out.image {
        content.push(json!({"type": "image", "data": STANDARD.encode(png), "mimeType": "image/png"}));
    }
    crate::bridge::hub(state, session_id).touch_activity();
    Some((content, is_error))
}

/// État du navigateur (onglet Navigateur des sessions).
pub async fn status(State(state): State<AppState>) -> Json<Value> {
    let b = &state.0.browser;
    if !b.enabled() {
        return Json(json!({"enabled": false}));
    }
    match b.version().await {
        Ok(v) => Json(json!({"enabled": true, "ok": true, "version": v["Browser"]})),
        Err(e) => Json(json!({"enabled": true, "ok": false, "error": e})),
    }
}

/// Fenêtre Regarder : WebSocket même origine.
pub async fn live(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    if !auth::origin_ok(&state, &headers) {
        return Err(ApiError::ForbiddenOrigin);
    }
    sessions::row(&state, &id).await?;
    if !state.0.browser.enabled() {
        return Err(ApiError::Invalid(
            "navigateur non configuré (ENGINEX_BROWSER_URL)".into(),
        ));
    }
    Ok(upgrade.on_upgrade(move |socket| run(state, id, socket)))
}

async fn send(ws: &mut futures_util::stream::SplitSink<WebSocket, Message>, v: Value) -> bool {
    ws.send(Message::Text(v.to_string().into())).await.is_ok()
}

async fn run(state: AppState, sid: String, socket: WebSocket) {
    let b = &state.0.browser;
    let (mut tx, mut rx) = socket.split();
    let mut tab = b.current(&sid).await;
    let _ = send(
        &mut tx,
        json!({"type": "status", "open": tab.is_some(), "taken": b.taken(&sid)}),
    )
    .await;
    loop {
        let Some(s) = tab.clone() else {
            // Pas d'onglet : on attend que Michael en ouvre un.
            match rx.next().await {
                Some(Ok(Message::Text(t))) => {
                    let v: Value = serde_json::from_str(&t).unwrap_or_default();
                    if v["type"] == "open" {
                        match b.get(&sid).await {
                            Ok(s) => tab = Some(s),
                            Err(e) => {
                                let _ = send(&mut tx, json!({"type": "error", "error": e})).await;
                            }
                        }
                    }
                    continue;
                }
                Some(Ok(_)) => continue,
                _ => return,
            }
        };
        let mut events = s.events.subscribe();
        if let Err(e) = enginex_browser::start_screencast(&s).await {
            let _ = send(&mut tx, json!({"type": "error", "error": e})).await;
            return;
        }
        let url = s.eval("location.href").await.ok().flatten().unwrap_or_default();
        let _ = send(
            &mut tx,
            json!({"type": "status", "open": true, "taken": b.taken(&sid), "url": url}),
        )
        .await;
        let mut check = tokio::time::interval(std::time::Duration::from_secs(2));
        let reason = loop {
            tokio::select! {
                _ = check.tick() => {
                    let same = b.current(&sid).await.is_some_and(|c| std::sync::Arc::ptr_eq(&c, &s));
                    if !same {
                        break "closed";
                    }
                }
                ev = events.recv() => {
                    let m = match ev {
                        Ok(m) => m,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(_) => break "closed",
                    };
                    let p = &m["params"];
                    if m["method"] == "Page.screencastFrame" {
                        s.send_nowait("Page.screencastFrameAck", json!({"sessionId": p["sessionId"]}));
                        let meta = &p["metadata"];
                        if !send(&mut tx, json!({"type": "frame", "data": p["data"],
                            "w": meta["deviceWidth"], "h": meta["deviceHeight"]})).await {
                            break "gone";
                        }
                    } else if m["method"] == "Page.frameNavigated" && p["frame"]["parentId"].is_null() {
                        let _ = send(&mut tx, json!({"type": "url", "url": p["frame"]["url"]})).await;
                    }
                }
                msg = rx.next() => {
                    let Some(Ok(Message::Text(t))) = msg else {
                        if matches!(msg, Some(Ok(_))) { continue; }
                        break "gone";
                    };
                    let v: Value = serde_json::from_str(&t).unwrap_or_default();
                    match v["type"].as_str().unwrap_or("") {
                        "take" => {
                            let on = v["on"].as_bool().unwrap_or(false);
                            b.set_taken(&sid, on);
                            let _ = send(&mut tx, json!({"type": "status", "open": true, "taken": on})).await;
                        }
                        "open" => {}
                        _ => {
                            if let Err(e) = enginex_browser::user_input(&s, &v).await {
                                let _ = send(&mut tx, json!({"type": "error", "error": e})).await;
                            }
                        }
                    }
                }
            }
        };
        enginex_browser::stop_screencast(&s).await;
        if reason == "gone" {
            return;
        }
        // Onglet fermé (sandbox arrêtée) : on attend un nouvel « open ».
        tab = None;
        let _ = send(
            &mut tx,
            json!({"type": "status", "open": false, "taken": b.taken(&sid)}),
        )
        .await;
    }
}
