//! Aperçu des apps : laissez-passer, relais HTTP et WebSocket vers l'app de
//! la sandbox (backend local : connexion directe à 127.0.0.1).

mod common;

use axum::{
    Json, Router,
    extract::ws::{Message as AxMsg, WebSocketUpgrade},
    http::{HeaderMap, Uri},
    routing::get,
};
use common::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

/// App de la sandbox : renvoie ce qu'elle a reçu, `/ws` en écho.
async fn app() -> u16 {
    let r = Router::new()
        .route(
            "/ws",
            get(|ws: WebSocketUpgrade| async move {
                ws.on_upgrade(|mut s| async move {
                    while let Some(Ok(AxMsg::Text(t))) = s.recv().await {
                        let _ = s.send(AxMsg::Text(format!("écho {t}").into())).await;
                    }
                })
            }),
        )
        .fallback(|uri: Uri, h: HeaderMap| async move {
            let s = |k: &str| h.get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
            (
                [("set-cookie", "enginex_session=pirate; Path=/"), ("x-app", "1")],
                Json(json!({"path": uri.to_string(), "host": s("host"), "cookie": s("cookie")})),
            )
        });
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(l, r).await.unwrap() });
    port
}

#[tokio::test]
async fn preview_serves_the_app_on_its_own_origin() {
    let (e, _fake) = env_sessions_cfg(&script_with("text", |_| {}), "", |c| {
        c.web.preview_bind = Some("127.0.0.1:0".parse().unwrap());
        c.web.preview_public_port = 8091;
    })
    .await;
    make_active_preset(&e).await;
    let sid = new_session(&e, "auto").await;

    // Sandbox arrêtée : rien à montrer.
    let st = e.get(&format!("/api/sessions/{sid}/preview")).await.body;
    assert_eq!(st["running"], false);

    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    turn(&e, &sid, "Bonjour").await;
    ev.until("turn_stored", |_| true).await;

    let port = app().await;
    let st = e.get(&format!("/api/sessions/{sid}/preview")).await.body;
    assert_eq!(st["running"], true, "{st}");
    assert!(
        st["ports"].as_array().unwrap().contains(&json!(port)),
        "port {port} absent : {st}"
    );

    let r = e
        .send("POST", &format!("/api/sessions/{sid}/preview"), json!({"port": port}))
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    // Même hôte que l'UI, port d'aperçu publié.
    let url = r.body["url"].as_str().unwrap();
    assert!(url.starts_with("http://enginex:8091/__enginex/open?g="), "{url}");
    let grant = url.split("g=").nth(1).unwrap().to_string();

    let base = serve_tcp(enginex_server::preview::router(e.state.clone())).await;
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    // Sans laissez-passer : refusé.
    let r = http.get(format!("{base}/")).send().await.unwrap();
    assert_eq!(r.status(), 403);
    let r = http
        .get(format!("{base}/__enginex/open?g=inconnu"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);

    let r = http
        .get(format!("{base}/__enginex/open?g={grant}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 302);
    assert_eq!(r.headers()["location"], "/");
    let set = r.headers()["set-cookie"].to_str().unwrap().to_string();
    assert!(
        set.starts_with(&format!("enginex_preview={grant};")) && set.contains("HttpOnly"),
        "{set}"
    );

    // Le cookie de session de l'UI part aussi vers ce port : jamais vers l'app.
    let cookie = format!("enginex_session=SECRET; theme=sombre; enginex_preview={grant}");
    let r = http
        .get(format!("{base}/src/main.ts?t=1"))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["x-app"], "1");
    assert!(
        r.headers().get("set-cookie").is_none(),
        "Set-Cookie enginex_* de l'app transmis"
    );
    let b: Value = r.json().await.unwrap();
    assert_eq!(b["path"], "/src/main.ts?t=1");
    assert_eq!(b["host"], format!("localhost:{port}"));
    assert_eq!(b["cookie"], "theme=sombre");

    // WebSocket (HMR de Vite) relayé.
    let mut req = format!("{}/ws", base.replace("http://", "ws://"))
        .into_client_request()
        .unwrap();
    req.headers_mut().insert("cookie", cookie.parse().unwrap());
    let (mut ws, _) = tokio_tungstenite::connect_async(req).await.unwrap();
    ws.send(Message::Text("salut".into())).await.unwrap();
    let m = ws.next().await.unwrap().unwrap();
    assert_eq!(m.into_text().unwrap().as_str(), "écho salut");

    // Port vide : message clair.
    let r = e
        .send(
            "POST",
            &format!("/api/sessions/{sid}/preview"),
            json!({"port": free_port()}),
        )
        .await;
    let g2 = r.body["url"].as_str().unwrap().split("g=").nth(1).unwrap().to_string();
    let r = http
        .get(format!("{base}/"))
        .header("cookie", format!("enginex_preview={g2}"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 502);
    assert!(r.text().await.unwrap().contains("ne répond pas"));

    // Session supprimée : laissez-passer retirés.
    let r = e.send("DELETE", &format!("/api/sessions/{sid}"), json!({})).await;
    assert_eq!(r.status, 200, "{}", r.text);
    let r = http
        .get(format!("{base}/"))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
}

#[tokio::test]
async fn preview_disabled() {
    let (e, _fake) = env_sessions(&script_with("text", |_| {})).await;
    let sid = new_session(&e, "auto").await;
    let st = e.get(&format!("/api/sessions/{sid}/preview")).await.body;
    assert_eq!(st["enabled"], false);
    let r = e
        .send("POST", &format!("/api/sessions/{sid}/preview"), json!({"port": 3000}))
        .await;
    assert_eq!(r.status, 400, "{}", r.text);
}

#[tokio::test]
async fn preview_serves_the_session_files_without_an_app() {
    let e = env_cfg("", |c| {
        c.web.preview_bind = Some("127.0.0.1:0".parse().unwrap());
        c.web.preview_public_port = 8091;
    })
    .await;
    let sid = new_session(&e, "auto").await;
    let dir = work_dir(&e, &sid).await;
    std::fs::write(
        dir.join("index.html"),
        "<h1>Le Fournil</h1><script src=app.js></script>",
    )
    .unwrap();
    std::fs::write(dir.join("app.js"), "console.log(1)").unwrap();
    std::fs::create_dir(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub/index.html"), "sous-page").unwrap();
    let outside = e.dir.path().join("secret.txt");
    std::fs::write(&outside, "SECRET").unwrap();
    std::os::unix::fs::symlink(&outside, dir.join("lien.txt")).unwrap();

    // Sandbox arrêtée, mais un index.html : la page est proposée.
    let st = e.get(&format!("/api/sessions/{sid}/preview")).await.body;
    assert_eq!(st["running"], false);
    assert_eq!(st["has_index"], true, "{st}");
    let r = e
        .send("POST", &format!("/api/sessions/{sid}/preview"), json!({"port": 0}))
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    let grant = r.body["url"].as_str().unwrap().split("g=").nth(1).unwrap().to_string();
    let base = serve_tcp(enginex_server::preview::router(e.state.clone())).await;
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let get = |p: &str| {
        http.get(format!("{base}{p}"))
            .header("cookie", format!("enginex_preview={grant}"))
            .send()
    };
    let r = get("/").await.unwrap();
    assert_eq!(r.status(), 200);
    assert!(r.headers()["content-type"].to_str().unwrap().starts_with("text/html"));
    assert!(r.text().await.unwrap().contains("Le Fournil"));
    let r = get("/app.js?v=2").await.unwrap();
    assert!(
        r.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/javascript")
    );
    // Dossier sans « / » : redirigé, puis son index.html.
    let r = get("/sub").await.unwrap();
    assert_eq!(r.status(), 302);
    assert_eq!(r.headers()["location"], "/sub/");
    assert_eq!(get("/sub/").await.unwrap().text().await.unwrap(), "sous-page");
    // Ni lien, ni remontée, ni .git, ni fichier absent.
    for p in ["/lien.txt", "/%2e%2e/secret.txt", "/.git/config", "/absent.html"] {
        let r = get(p).await.unwrap();
        assert_eq!(r.status(), 404, "{p}");
        assert!(!r.text().await.unwrap().contains("SECRET"), "{p}");
    }
}
