//! Outils web du MCP intégré : `web_search` (faux SearXNG) et `web_read`
//! (pages servies par le test, refus des adresses privées).

mod common;

use axum::{
    Json, Router,
    extract::Query,
    http::{StatusCode, header},
    response::{IntoResponse, Redirect},
    routing::get,
};
use common::{env_cfg, mcp_tool, new_session, sandbox_key, serve_tcp};
use serde_json::json;
use std::collections::HashMap;

async fn fake_web() -> String {
    let page = r#"<html><head><title>Doc</title><script>alert(1)</script></head>
<body><h1>Installer EngineX</h1><p>Le port par défaut est <b>8090</b>.</p>
<ul><li>un</li><li>deux</li></ul><a href="/suite">suite</a></body></html>"#;
    let long = format!("<p>{}</p>", "mot ".repeat(10_000));
    let router = Router::new()
        .route(
            "/search",
            get(|Query(q): Query<HashMap<String, String>>| async move {
                if q.get("format").map(String::as_str) != Some("json") {
                    return StatusCode::FORBIDDEN.into_response();
                }
                let term = q.get("q").cloned().unwrap_or_default();
                Json(json!({"query": term, "results": [
                    {"title": format!("Résultat {term}"), "url": "https://exemple.org/a", "content": "premier extrait"},
                    {"title": "Deuxième", "url": "https://exemple.org/b", "content": "second extrait"},
                    {"title": "Sans adresse", "content": "ignoré"},
                ]}))
                .into_response()
            }),
        )
        .route(
            "/page",
            get(move || async move { ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], page) }),
        )
        .route(
            "/long",
            get(move || async move { ([(header::CONTENT_TYPE, "text/html")], long.clone()) }),
        )
        .route("/redir", get(|| async { Redirect::temporary("/page") }))
        .route(
            "/pdf",
            get(|| async { ([(header::CONTENT_TYPE, "application/pdf")], "%PDF-1.7") }),
        )
        .route("/data.json", get(|| async { Json(json!({"ok": true})) }));
    serve_tcp(router).await
}

#[tokio::test]
async fn search_and_read() {
    let base = fake_web().await;
    let b = base.clone();
    let e = env_cfg("", move |c| c.web.searxng_url = b).await;
    let sid = new_session(&e, "auto").await;
    let key = sandbox_key(&e, &sid).await;

    let (t, err) = mcp_tool(&e, &key, "web_search", json!({"query": "enginex port"})).await;
    assert!(!err, "{t}");
    assert!(
        t.starts_with("1. Résultat enginex port\n   https://exemple.org/a\n   premier extrait"),
        "{t}"
    );
    assert!(t.contains("2. Deuxième") && !t.contains("Sans adresse"), "{t}");
    let (t, _) = mcp_tool(&e, &key, "web_search", json!({"query": "x", "count": 1})).await;
    assert!(!t.contains("Deuxième"), "{t}");

    let (t, err) = mcp_tool(&e, &key, "web_read", json!({"url": format!("{base}/page")})).await;
    assert!(!err, "{t}");
    assert!(t.contains("Type : text/html"), "{t}");
    assert!(
        t.contains("Installer EngineX") && t.contains("8090") && t.contains("deux"),
        "{t}"
    );
    assert!(!t.contains("alert(1)") && !t.contains("<p>"), "{t}");
    // Redirection suivie, adresse finale donnée.
    let (t, err) = mcp_tool(&e, &key, "web_read", json!({"url": format!("{base}/redir")})).await;
    assert!(!err && t.contains(&format!("Adresse : {base}/page")), "{t}");
    // Texte coupé, signalé.
    let (t, _) = mcp_tool(
        &e,
        &key,
        "web_read",
        json!({"url": format!("{base}/long"), "max_chars": 1000}),
    )
    .await;
    assert!(
        t.contains("[… texte coupé : 1000 caractères"),
        "{}",
        &t[t.len() - 200..]
    );
    // JSON tel quel ; PDF refusé ; schéma non http refusé.
    let (t, _) = mcp_tool(&e, &key, "web_read", json!({"url": format!("{base}/data.json")})).await;
    assert!(t.contains("{\"ok\":true}"), "{t}");
    let (t, err) = mcp_tool(&e, &key, "web_read", json!({"url": format!("{base}/pdf")})).await;
    assert!(err && t.contains("application/pdf"), "{t}");
    let (t, err) = mcp_tool(&e, &key, "web_read", json!({"url": "file:///etc/passwd"})).await;
    assert!(err && t.contains("schéma file"), "{t}");
}

#[tokio::test]
async fn private_addresses_refused() {
    let base = fake_web().await;
    let e = env_cfg("", |c| c.web.allow_private = false).await;
    let sid = new_session(&e, "auto").await;
    let key = sandbox_key(&e, &sid).await;
    for url in [
        format!("{base}/page"),
        "http://localhost:2375/containers/json".to_string(),
        "http://10.0.0.1/".to_string(),
        "http://169.254.169.254/latest/meta-data/".to_string(),
        "http://[::1]:8090/".to_string(),
        "http://[::ffff:192.168.1.1]/".to_string(),
    ] {
        let (t, err) = mcp_tool(&e, &key, "web_read", json!({"url": url})).await;
        assert!(err && t.contains("adresse refusée"), "{url} : {t}");
    }
    // SearXNG injoignable : message clair, pas de panne.
    let (t, err) = mcp_tool(&e, &key, "web_search", json!({"query": "x"})).await;
    assert!(err && t.contains("SearXNG"), "{t}");
}
