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

/// App de la sandbox : un compteur, l'hôte reçu, une erreur de console et une
/// ressource absente.
async fn counter_app() -> u16 {
    use axum::{Router, http::HeaderMap, response::Html, routing::get};
    let r = Router::new().route(
        "/",
        get(|h: HeaderMap| async move {
            let host = h.get("host").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
            Html(format!(
                "<!doctype html><title>Essai</title><h1>Compteur : <span id=n>0</span></h1><p>Hôte : {host}</p>\
                 <button onclick=\"n.textContent=+n.textContent+1\">Ajouter</button>\
                 <script src=\"/absent.js\"></script><script>console.error('oups', 42)</script>"
            ))
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(l, r).await.unwrap() });
    port
}

/// Le sas sans navigateur : seul le pair « navigateur » passe, l'app reçoit
/// `Host: localhost:<port>`, et le sas tombe avec la session.
#[tokio::test]
async fn app_gate_serves_the_browser_only() {
    // Le « navigateur » : une simple écoute sur une autre adresse de bouclage.
    let chrome = tokio::net::TcpListener::bind("127.0.0.2:0").await.unwrap();
    let url = format!("http://{}", chrome.local_addr().unwrap());
    let (e, _fake) = env_sessions_cfg(&script_with("text", |_| {}), "", |c| c.web.browser_url = Some(url)).await;
    make_active_preset(&e).await;
    let sid = new_session(&e, "auto").await;
    let app = counter_app().await;
    let stopped = enginex_server::appgate::open_app(&e.state, &sid, app).await;
    assert!(stopped.unwrap_err().contains("sandbox de la session est arrêtée"));

    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    turn(&e, &sid, "Bonjour").await;
    ev.until("turn_stored", |_| true).await;
    let origin = enginex_server::appgate::open_app(&e.state, &sid, app).await.unwrap();
    // Même (session, port) : même sas.
    assert_eq!(
        enginex_server::appgate::open_app(&e.state, &sid, app).await.unwrap(),
        origin
    );
    assert_eq!(e.state.gates().port_of(&sid, &origin), Some(app));

    let from = |ip: [u8; 4]| {
        reqwest::Client::builder()
            .local_address(std::net::IpAddr::from(ip))
            .build()
            .unwrap()
    };
    let r = from([127, 0, 0, 2]).get(&origin).send().await.unwrap();
    assert_eq!(r.status(), 200);
    let body = r.text().await.unwrap();
    assert!(body.contains(&format!("Hôte : localhost:{app}")), "{body}");
    // Tout autre pair (une autre sandbox, le réseau local) est refusé.
    let r = from([127, 0, 0, 3]).get(&origin).send().await.unwrap();
    assert_eq!(r.status(), 403);

    e.state.gates().close_session(&sid);
    assert!(!e.state.gates().has(&sid));
    assert!(from([127, 0, 0, 2]).get(&origin).send().await.is_err());
}

/// Vrai Chromium : l'agent ouvre l'app de sa sandbox par `localhost`, lit le
/// texte, la console et la capture, puis clique.
#[tokio::test]
#[ignore = "lancé par la CI avec un vrai Chromium (ENGINEX_TEST_BROWSER)"]
async fn real_chromium_opens_the_sandbox_app() {
    let url = std::env::var("ENGINEX_TEST_BROWSER").expect("ENGINEX_TEST_BROWSER");
    let (e, _fake) = env_sessions_cfg(&script_with("text", |_| {}), "", |c| c.web.browser_url = Some(url)).await;
    make_active_vision_preset(&e).await;
    let sid = new_session(&e, "auto").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    turn(&e, &sid, "Bonjour").await;
    ev.until("turn_stored", |_| true).await;
    // La sandbox vivante a déjà sa clé : le test en prend la place.
    let key = format!("ex_test{}", uuid::Uuid::new_v4().simple());
    let done = sqlx::query("UPDATE api_keys SET hash = ? WHERE name = ?")
        .bind(<sha2::Sha256 as sha2::Digest>::digest(key.as_bytes()).to_vec())
        .bind(format!("sandbox:{sid}"))
        .execute(e.state.db())
        .await
        .unwrap();
    assert_eq!(done.rows_affected(), 1);

    // Rien n'écoute : message clair, pas de page d'erreur du navigateur.
    let dead = free_port();
    let (text, err) = mcp_tool(
        &e,
        &key,
        "browser_open",
        json!({"url": format!("http://localhost:{dead}")}),
    )
    .await;
    assert!(
        err && text.contains(&format!("rien n'écoute sur le port {dead}")),
        "{text}"
    );

    let port = counter_app().await;
    let r = rpc(
        &e,
        &key,
        "tools/call",
        json!({"name": "browser_open", "arguments": {"url": format!("localhost:{port}")}}),
    )
    .await;
    let content = r["result"]["content"].as_array().unwrap();
    let text = content[0]["text"].as_str().unwrap();
    assert_eq!(r["result"]["isError"], false, "{text}");
    eprintln!("--- browser_open (app) ---\n{text}\n---");
    // L'adresse du sas ne paraît jamais : le modèle lit celle qu'il a écrite.
    assert!(text.contains(&format!("http://localhost:{port}/")), "{text}");
    assert!(!text.contains("http://127.0.0.1"), "{text}");
    assert!(text.contains("button \"Ajouter\""), "{text}");
    assert!(
        text.contains("Texte visible de la page :") && text.contains("Compteur : 0"),
        "{text}"
    );
    // L'app reçoit `Host: localhost:<port>` (Vite refuse les autres hôtes).
    assert!(text.contains(&format!("Hôte : localhost:{port}")), "{text}");
    assert!(text.contains("console.error : oups 42"), "{text}");
    assert!(text.contains("404") && text.contains("/absent.js"), "{text}");
    // Preset avec vision : la capture est jointe d'office.
    assert!(text.contains("Capture jointe"), "{text}");
    assert_eq!(content[1]["type"], "image", "{r}");
    assert!(content[1]["data"].as_str().unwrap().len() > 1000);

    let (text, err) = mcp_tool(&e, &key, "browser_click", json!({"ref": 1})).await;
    assert!(!err && text.contains("Compteur : 1"), "{text}");
    assert!(!text.contains("console.error"), "erreurs déjà rendues : {text}");
    let (text, _) = mcp_tool(&e, &key, "browser_snapshot", json!({})).await;
    assert!(text.contains("(texte visible inchangé)"), "{text}");

    // Hors de l'app : ni texte ni console ajoutés.
    let (text, _) = mcp_tool(
        &e,
        &key,
        "browser_open",
        json!({"url": "data:text/html,%3Ctitle%3EA%3C/title%3E%3Cp%3EAilleurs%3C/p%3E"}),
    )
    .await;
    assert!(!text.contains("Texte visible"), "{text}");
}
