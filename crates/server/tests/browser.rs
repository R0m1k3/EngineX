//! Navigateur de la stack : outils MCP `browser_*` et fenêtre Regarder.

mod common;

use axum::{
    body::Body,
    http::{Request, header},
};
use common::*;
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt;

async fn rpc(e: &Env, key: &str, method: &str, params: Value) -> Value {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    let req = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header(header::HOST, "enginex:8090")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .body(Body::from(body.to_string()))
        .unwrap();
    let res = e.router.clone().oneshot(req).await.unwrap();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn names(list: &Value) -> Vec<String> {
    list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn tools_listed_only_when_configured() {
    let e = env().await;
    let sid = new_session(&e, "auto").await;
    let key = sandbox_key(&e, &sid).await;
    let n = names(&rpc(&e, &key, "tools/list", json!({})).await);
    assert!(!n.iter().any(|t| t.starts_with("browser_")), "{n:?}");
    let r = rpc(
        &e,
        &key,
        "tools/call",
        json!({"name": "browser_open", "arguments": {"url": "x"}}),
    )
    .await;
    assert!(r["error"]["message"].as_str().unwrap().contains("inconnu"), "{r}");
    assert_eq!(e.get("/api/browser").await.body["enabled"], false);

    let e = env_cfg("", |c| c.web.browser_url = Some("http://127.0.0.1:9".into())).await;
    let sid = new_session(&e, "auto").await;
    let key = sandbox_key(&e, &sid).await;
    let n = names(&rpc(&e, &key, "tools/list", json!({})).await);
    for t in [
        "browser_open",
        "browser_snapshot",
        "browser_click",
        "browser_screenshot",
    ] {
        assert!(n.contains(&t.to_string()), "{t} absent : {n:?}");
    }
    // Navigateur injoignable : erreur lisible, pas de panne.
    let (text, err) = mcp_tool(&e, &key, "browser_open", json!({"url": "https://example.org"})).await;
    assert!(err && text.contains("injoignable"), "{text}");
    let st = e.get("/api/browser").await.body;
    assert_eq!(st["enabled"], true);
    assert_eq!(st["ok"], false);
}

/// Vrai Chromium (`ENGINEX_TEST_BROWSER`, job CI) : outils MCP, image de la
/// capture, Regarder (images, prise de main qui bloque les outils, clic de
/// Michael).
#[tokio::test]
#[ignore = "lancé par la CI avec un vrai Chromium (ENGINEX_TEST_BROWSER)"]
async fn real_chromium_through_mcp_and_watch() {
    let url = std::env::var("ENGINEX_TEST_BROWSER").expect("ENGINEX_TEST_BROWSER");
    let e = env_cfg("", |c| c.web.browser_url = Some(url)).await;
    let sid = new_session(&e, "auto").await;
    let key = sandbox_key(&e, &sid).await;
    assert_eq!(e.get("/api/browser").await.body["ok"], true);

    let page = "data:text/html,%3Ctitle%3EA%3C/title%3E%3Cbutton%20onclick=%22document.title=%27clic%27%22%3EBouton%3C/button%3E";
    let (text, err) = mcp_tool(&e, &key, "browser_open", json!({"url": page})).await;
    assert!(!err && text.contains("Bouton"), "{text}");

    // Capture : image MCP (le preset actif n'a pas la vision ici).
    let r = rpc(
        &e,
        &key,
        "tools/call",
        json!({"name": "browser_screenshot", "arguments": {}}),
    )
    .await;
    assert_eq!(r["result"]["isError"], true, "{r}");
    assert!(
        r["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("ne voit pas les images")
    );

    // Regarder.
    let base = serve_tcp(e.router.clone()).await;
    let mut ws = ws_connect(
        &base,
        &e.cookie,
        &format!("/api/sessions/{sid}/browser/live"),
        Some(&base),
    )
    .await
    .expect("Regarder ouvert");
    let mut frame = false;
    let mut status_open = false;
    for _ in 0..50 {
        let m = tokio::time::timeout(std::time::Duration::from_secs(10), ws.next())
            .await
            .expect("message")
            .unwrap()
            .unwrap();
        let Message::Text(t) = m else { continue };
        let v: Value = serde_json::from_str(&t).unwrap();
        if v["type"] == "status" && v["open"] == true {
            status_open = true;
        }
        if v["type"] == "frame" {
            assert!(v["data"].as_str().unwrap().len() > 100);
            frame = true;
            break;
        }
    }
    assert!(status_open && frame, "statut et image attendus");

    // Prise de main : les outils attendent.
    ws.send(Message::Text(json!({"type": "take", "on": true}).to_string().into()))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let (text, _) = mcp_tool(&e, &key, "browser_snapshot", json!({})).await;
    assert!(text.starts_with("[attente]"), "{text}");
    // Clic de Michael sur le bouton (en haut à gauche de la page).
    for action in ["down", "up"] {
        ws.send(Message::Text(
            json!({"type": "mouse", "action": action, "x": 30, "y": 18})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    }
    ws.send(Message::Text(json!({"type": "take", "on": false}).to_string().into()))
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let (text, err) = mcp_tool(&e, &key, "browser_snapshot", json!({})).await;
    assert!(
        !err && text.contains("clic"),
        "titre changé par le clic de Michael attendu : {text}"
    );

    // Sandbox arrêtée : onglet fermé.
    enginex_server::sandboxes::stop(&e.state, &sid).await;
    assert!(e.state.browser().current(&sid).await.is_none());
}
