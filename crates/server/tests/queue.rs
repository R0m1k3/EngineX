//! File d'attente commune des tours (étape 7a) : ordre des baux, changement
//! de modèle, clés externes, tours des sessions sur un autre preset.

mod common;

use std::time::Duration;

use axum::{
    body::Body,
    http::{Request, header},
};
use common::*;
use enginex_server::queue::{Ask, BACKGROUND, Lease, USER, Who};
use http_body_util::BodyExt;
use serde_json::json;
use tokio::sync::mpsc;
use tower::ServiceExt;

fn ask(who: &str, priority: u8, preset: Option<i64>) -> Ask {
    Ask {
        who: Who::Session(who.into()),
        priority,
        preset,
        label: who.into(),
    }
}

/// Demande en fond ; le nom part dans `done` quand le bail est donné.
fn spawn_ask(e: &Env, a: Ask, done: mpsc::UnboundedSender<(String, Lease)>) {
    let state = e.state.clone();
    tokio::spawn(async move {
        let name = a.label.clone();
        let l = state
            .queue()
            .acquire(&state, a, &|_| {}, std::future::pending())
            .await
            .expect("bail");
        let _ = done.send((name, l));
    });
}

async fn next(rx: &mut mpsc::UnboundedReceiver<(String, Lease)>) -> (String, Lease) {
    tokio::time::timeout(Duration::from_secs(30), rx.recv())
        .await
        .expect("bail attendu")
        .unwrap()
}

async fn waiting(e: &Env, n: usize) {
    e.until("/api/queue", |v| v["waiting"].as_array().unwrap().len() == n)
        .await;
}

#[tokio::test]
async fn michael_first_then_model_swap() {
    let e = env().await;
    let p1 = make_active_preset(&e).await;
    let r = e
        .send("POST", "/api/presets", json!({"from": p1, "name": "autre"}))
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    let p2 = r.body["id"].as_i64().unwrap();
    let q = e.state.queue();

    // Un slot (-np 1) : le premier passe, les autres attendent.
    let a = q
        .acquire(
            &e.state,
            ask("a", BACKGROUND, Some(p1)),
            &|_| {},
            std::future::pending(),
        )
        .await
        .unwrap();
    let (tx, mut rx) = mpsc::unbounded_channel();
    spawn_ask(&e, ask("fond", BACKGROUND, Some(p1)), tx.clone());
    waiting(&e, 1).await;
    spawn_ask(&e, ask("michael", USER, Some(p1)), tx.clone());
    waiting(&e, 2).await;
    let snap = e.get("/api/queue").await.body;
    assert_eq!(snap["holders"][0]["label"], "a");
    assert_eq!(snap["waiting"][0]["label"], "michael", "{snap}");
    assert_eq!(snap["loaded"], p1);

    // Michael passe devant le cron arrivé avant lui.
    drop(a);
    let (n, l) = next(&mut rx).await;
    assert_eq!(n, "michael");
    drop(l);
    let (n, l) = next(&mut rx).await;
    assert_eq!(n, "fond");

    // Une demande sur un autre preset attend les baux, puis le charge ; à
    // priorité égale, celles du modèle chargé passent avant.
    spawn_ask(&e, ask("p2", BACKGROUND, Some(p2)), tx.clone());
    waiting(&e, 1).await;
    spawn_ask(&e, ask("p1-tard", BACKGROUND, Some(p1)), tx.clone());
    waiting(&e, 2).await;
    let snap = e.get("/api/queue").await.body;
    assert_eq!(snap["waiting"][0]["label"], "p1-tard", "{snap}");
    assert_eq!(snap["waiting"][1]["needs_swap"], true);
    drop(l);
    let (n, l) = next(&mut rx).await;
    assert_eq!(n, "p1-tard");
    assert_eq!(e.get("/api/engine").await.body["state"]["preset_id"], p1);
    drop(l);
    let (n, l) = next(&mut rx).await;
    assert_eq!(n, "p2");
    let eng = e.get("/api/engine").await.body;
    assert_eq!(eng["state"]["state"], "ready", "{eng}");
    assert_eq!(eng["state"]["preset_id"], p2);
    // Preset par défaut inchangé (choix de Michael), preset chargé = p2.
    let st = e.state.clone();
    assert_eq!(enginex_server::presets::default_id(&st).await.unwrap(), Some(p1));
    drop(l);
    e.until("/api/queue", |v| v["holders"].as_array().unwrap().is_empty())
        .await;
}

#[tokio::test]
async fn external_key_waits_for_the_turn() {
    let e = env().await;
    let p1 = make_active_preset(&e).await;
    let key = new_key(&e, "pc").await;
    let held = e
        .state
        .queue()
        .acquire(&e.state, ask("tour", USER, Some(p1)), &|_| {}, std::future::pending())
        .await
        .unwrap();
    let router = e.router.clone();
    let call = tokio::spawn(async move {
        let req = Request::builder()
            .method("POST")
            .uri("/v1/chat/completions")
            .header(header::HOST, "enginex:8090")
            .header(header::AUTHORIZATION, format!("Bearer {key}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                json!({"model": "x", "messages": [{"role": "user", "content": "salut"}]}).to_string(),
            ))
            .unwrap();
        let res = router.oneshot(req).await.unwrap();
        let status = res.status();
        let _ = res.into_body().collect().await.unwrap();
        status
    });
    waiting(&e, 1).await;
    let snap = e.get("/api/queue").await.body;
    assert_eq!(snap["waiting"][0]["label"], "clé d'API « pc »");
    assert!(!call.is_finished());
    drop(held);
    let status = tokio::time::timeout(Duration::from_secs(30), call)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status, 200);
    e.until("/api/queue", |v| v["holders"].as_array().unwrap().is_empty())
        .await;
}

#[tokio::test]
async fn session_on_another_preset_loads_it() {
    let (e, _fake) = env_sessions(&script_with("text", |_| {})).await;
    let p1 = make_active_preset(&e).await;
    let r = e
        .send("POST", "/api/presets", json!({"from": p1, "name": "autre"}))
        .await;
    let p2 = r.body["id"].as_i64().unwrap();
    let sid = new_session(&e, "auto").await;
    let r = e
        .send("PATCH", &format!("/api/sessions/{sid}"), json!({"preset_id": p2}))
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    let r = e
        .send("PATCH", &format!("/api/sessions/{sid}"), json!({"preset_id": 999}))
        .await;
    assert_eq!(r.status, 400);
    assert_eq!(
        e.get(&format!("/api/sessions/{sid}")).await.body["session"]["preset_id"],
        p2
    );

    // Un tour de fond tient le modèle : la session attend, l'attente est montrée.
    let held = e
        .state
        .queue()
        .acquire(
            &e.state,
            ask("cron", BACKGROUND, Some(p1)),
            &|_| {},
            std::future::pending(),
        )
        .await
        .unwrap();
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    let r = turn(&e, &sid, "Bonjour").await;
    assert_eq!(r.status, 202, "{}", r.text);
    let w = ev.until("queue", |d| !d["wait"].is_null()).await;
    assert_eq!(w["wait"]["running"][0]["label"], "cron", "{w}");
    assert_eq!(w["wait"]["needs_swap"], true);
    drop(held);
    ev.until("turn_stored", |_| true).await;
    let eng = e.get("/api/engine").await.body;
    assert_eq!(eng["state"]["preset_id"], p2, "{eng}");
    // Bail rendu à la fin du tour.
    e.until("/api/queue", |v| v["holders"].as_array().unwrap().is_empty())
        .await;

    // Retour au preset par défaut : la session suivante recharge p1.
    let r = e
        .send("PATCH", &format!("/api/sessions/{sid}"), json!({"preset_id": null}))
        .await;
    assert_eq!(r.status, 200);
    assert!(e.get(&format!("/api/sessions/{sid}")).await.body["session"]["preset_id"].is_null());
}

#[tokio::test]
async fn queued_turn_can_be_cancelled() {
    let (e, _fake) = env_sessions(&script_with("text", |_| {})).await;
    let p1 = make_active_preset(&e).await;
    let sid = new_session(&e, "auto").await;
    let held = e
        .state
        .queue()
        .acquire(
            &e.state,
            ask("cron", BACKGROUND, Some(p1)),
            &|_| {},
            std::future::pending(),
        )
        .await
        .unwrap();
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    turn(&e, &sid, "Bonjour").await;
    ev.until("queue", |d| !d["wait"].is_null()).await;
    let r = e
        .send("POST", &format!("/api/sessions/{sid}/interrupt"), json!({}))
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    let t = ev.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["status"], "failed", "{t}");
    assert!(t["turn"]["error"].as_str().unwrap().contains("file"), "{t}");
    waiting(&e, 0).await;
    drop(held);
}

#[tokio::test]
async fn vision_checked_on_the_session_preset() {
    let (e, _fake) = env_sessions(&script_with("text", |_| {})).await;
    let plain = make_active_preset(&e).await;
    let vision = make_active_vision_preset(&e).await;
    // Chargé : vision ; session sur le preset sans vision → image refusée.
    let sid = new_session(&e, "auto").await;
    e.send("PATCH", &format!("/api/sessions/{sid}"), json!({"preset_id": plain}))
        .await;
    let up = upload(&e, &format!("/api/sessions/{sid}/attachments"), "image/png", png(4, 4)).await;
    assert_eq!(up.status, 200, "{}", up.text);
    let r = e
        .send(
            "POST",
            &format!("/api/sessions/{sid}/turns"),
            json!({"text": "regarde", "attachments": [up.body["id"]]}),
        )
        .await;
    assert_eq!(r.status, 409, "{}", r.text);
    assert_eq!(r.body["error"], "no_vision", "{}", r.text);
    let _ = vision;
}
