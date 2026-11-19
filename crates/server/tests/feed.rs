//! Fil de notifications, webhooks entrants et push web (étape 7b).

mod common;

use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    body::Bytes,
    http::{HeaderMap, StatusCode},
    routing::post,
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use common::*;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use web_push_native::{
    Auth,
    p256::{SecretKey, elliptic_curve::rand_core::OsRng, elliptic_curve::sec1::ToEncodedPoint},
};

async fn hook(e: &Env, path: &str, headers: &[(&str, &str)], body: &str) -> (u16, Value) {
    let mut rb = reqwest::Client::new()
        .post(format!("{}{path}", e.base().await))
        .body(body.to_string());
    for (k, v) in headers {
        rb = rb.header(*k, *v);
    }
    let r = rb.send().await.unwrap();
    let st = r.status().as_u16();
    (st, r.json().await.unwrap_or(Value::Null))
}

trait Base {
    async fn base(&self) -> String;
}
impl Base for Env {
    async fn base(&self) -> String {
        serve_tcp(self.router.clone()).await
    }
}

#[tokio::test]
async fn webhook_writes_to_the_feed() {
    let e = env().await;
    let r = e
        .send("POST", "/api/webhooks", json!({"name": "ci", "level": "warning"}))
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    let path = r.body["path"].as_str().unwrap().to_string();
    let id = r.body["id"].as_i64().unwrap();
    // Le jeton n'est jamais relu.
    let list = e.get("/api/webhooks").await.body;
    assert!(!list.to_string().contains(path.trim_start_matches("/hooks/")));

    let mut ev = sse(&e, "/api/feed/events").await;
    ev.until("feed", |d| d["unread"] == 0).await;

    let push = json!({"ref": "refs/heads/main", "repository": {"full_name": "michael/app"},
        "commits": [{"message": "corrige le menu\n\ndétails"}, {"message": "ajoute un test"}]});
    let (st, _) = hook(
        &e,
        &path,
        &[("x-github-event", "push"), ("content-type", "application/json")],
        &push.to_string(),
    )
    .await;
    assert_eq!(st, 202);
    let d = ev.until("feed", |d| d["item"].is_object()).await;
    assert_eq!(d["item"]["title"], "ci : push sur michael/app (main) : 2 commit(s)");
    assert_eq!(d["item"]["level"], "warning");
    assert!(d["item"]["body"].as_str().unwrap().contains("- corrige le menu"));
    assert_eq!(d["unread"], 1);

    // Texte brut et JSON libre.
    let (st, _) = hook(&e, &path, &[], "Sauvegarde finie\n3 Go").await;
    assert_eq!(st, 202);
    let (st, _) = hook(
        &e,
        &path,
        &[],
        &json!({"title": "Vente", "message": "12 €"}).to_string(),
    )
    .await;
    assert_eq!(st, 202);
    let f = e.get("/api/feed").await.body;
    assert_eq!(f["unread"], 3);
    assert_eq!(f["items"][0]["title"], "ci : Vente");
    assert_eq!(f["items"][0]["body"], "12 €");
    assert_eq!(f["items"][1]["title"], "ci : Sauvegarde finie");
    assert_eq!(e.get("/api/webhooks").await.body[0]["count"], 3);

    // Lu, supprimé.
    let first = f["items"][0]["id"].as_i64().unwrap();
    let r = e.send("POST", "/api/feed/read", json!({"ids": [first]})).await;
    assert_eq!(r.body["unread"], 2);
    let r = e.send("POST", "/api/feed/read", json!({"all": true})).await;
    assert_eq!(r.body["unread"], 0);
    let r = e.send("DELETE", &format!("/api/feed/{first}"), json!({})).await;
    assert_eq!(r.status, 200);
    assert_eq!(e.get("/api/feed").await.body["items"].as_array().unwrap().len(), 2);

    // Inconnu, désactivé, trop gros, trop d'appels.
    let (st, _) = hook(&e, "/hooks/inconnu", &[], "x").await;
    assert_eq!(st, 404);
    let (st, _) = hook(&e, &path, &[], &"x".repeat(70 * 1024)).await;
    assert_eq!(st, 413);
    let mut last = 0;
    for _ in 0..30 {
        last = hook(&e, &path, &[], "x").await.0;
    }
    assert_eq!(last, 429);
    e.send("PATCH", &format!("/api/webhooks/{id}"), json!({"enabled": false}))
        .await;
    let (st, _) = hook(&e, &path, &[], "x").await;
    assert_eq!(st, 403);
    let r = e.send("DELETE", &format!("/api/webhooks/{id}"), json!({})).await;
    assert_eq!(r.status, 200);
    let (st, _) = hook(&e, &path, &[], "x").await;
    assert_eq!(st, 404);

    let r = e.send("DELETE", "/api/feed", json!({})).await;
    assert_eq!(r.body["unread"], 0);
    assert!(e.get("/api/feed").await.body["items"].as_array().unwrap().is_empty());
}

type Seen = Arc<Mutex<Vec<(HeaderMap, Bytes)>>>;

/// Faux service de push : garde chaque requête, répond `status`.
async fn push_service(status: StatusCode) -> (String, Seen) {
    let seen: Seen = Arc::default();
    let s = seen.clone();
    let r = Router::new().route(
        "/push/{id}",
        post(move |h: HeaderMap, b: Bytes| {
            let s = s.clone();
            async move {
                s.lock().await.push((h, b));
                status
            }
        }),
    );
    (serve_tcp(r).await, seen)
}

#[tokio::test]
async fn push_is_encrypted_for_the_browser() {
    let e = env().await;
    let st = e.get("/api/push").await.body;
    let key = URL_SAFE_NO_PAD.decode(st["public_key"].as_str().unwrap()).unwrap();
    assert_eq!(key.len(), 65, "point non compressé attendu");
    assert_eq!(st["mode"], "important");
    // Même clé après relecture (fichier vapid.key).
    assert_eq!(e.get("/api/push").await.body["public_key"], st["public_key"]);

    // Navigateur : clé et secret d'abonnement.
    let ua = SecretKey::random(&mut OsRng);
    let p256dh = URL_SAFE_NO_PAD.encode(ua.public_key().to_encoded_point(false).as_bytes());
    let auth = [7u8; 16];
    let (base, seen) = push_service(StatusCode::CREATED).await;
    let r = e
        .send(
            "POST",
            "/api/push/subscribe",
            json!({"endpoint": format!("{base}/push/1"), "keys": {"p256dh": p256dh, "auth": URL_SAFE_NO_PAD.encode(auth)}}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    // http refusé hors de la machine elle-même.
    let r = e
        .send(
            "POST",
            "/api/push/subscribe",
            json!({"endpoint": "http://exemple.org/p", "keys": {"p256dh": p256dh, "auth": URL_SAFE_NO_PAD.encode(auth)}}),
        )
        .await;
    assert_eq!(r.status, 400);

    let r = e.send("POST", "/api/push/test", json!({})).await;
    assert_eq!(r.body["sent"], 1, "{}", r.text);
    let (h, body) = seen.lock().await.pop().unwrap();
    assert_eq!(h["content-encoding"], "aes128gcm");
    assert!(h["authorization"].to_str().unwrap().starts_with("vapid t="));
    let clear = web_push_native::decrypt(body.to_vec(), &ua, Auth::from_slice(&auth)).unwrap();
    let v: Value = serde_json::from_slice(&clear).unwrap();
    assert!(v["body"].as_str().unwrap().contains("essai"));

    // Mode « important » : une notification ordinaire ne part pas, une erreur oui.
    let r = e
        .send("POST", "/api/webhooks", json!({"name": "n", "push": false}))
        .await;
    let quiet = r.body["path"].as_str().unwrap().to_string();
    let r = e
        .send("POST", "/api/webhooks", json!({"name": "alerte", "push": true}))
        .await;
    let loud = r.body["path"].as_str().unwrap().to_string();
    hook(&e, &quiet, &[], "rien").await;
    hook(&e, &loud, &[], "important").await;
    for _ in 0..100 {
        if !seen.lock().await.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let got = std::mem::take(&mut *seen.lock().await);
    assert_eq!(got.len(), 1);
    let clear = web_push_native::decrypt(got[0].1.to_vec(), &ua, Auth::from_slice(&auth)).unwrap();
    let v: Value = serde_json::from_slice(&clear).unwrap();
    assert_eq!(v["title"], "alerte : important");

    // Mode « off » : rien ne part.
    let r = e.send("PUT", "/api/push/mode", json!({"mode": "off"})).await;
    assert_eq!(r.status, 200);
    hook(&e, &loud, &[], "encore").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(seen.lock().await.is_empty());

    // Abonnement expiré (410) : retiré.
    let (gone, _) = push_service(StatusCode::GONE).await;
    e.send(
        "POST",
        "/api/push/subscribe",
        json!({"endpoint": format!("{gone}/push/2"), "keys": {"p256dh": p256dh, "auth": URL_SAFE_NO_PAD.encode(auth)}}),
    )
    .await;
    assert_eq!(
        e.get("/api/push").await.body["subscriptions"].as_array().unwrap().len(),
        2
    );
    let r = e.send("POST", "/api/push/test", json!({})).await;
    assert_eq!(r.body["failed"], 1, "{}", r.text);
    assert_eq!(
        e.get("/api/push").await.body["subscriptions"].as_array().unwrap().len(),
        1
    );
}
