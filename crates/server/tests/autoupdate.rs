//! Mise à jour automatique de llama.cpp : registre local, `tick` appelé à la
//! main (pas de boucle de fond dans les tests).

mod common;

use std::time::Duration;

use axum::http::StatusCode;
use common::{Env, env, env_cfg, env_with, make_active_preset, new_key, publish, serve_tcp};
use enginex_server::autoupdate::tick;
use serde_json::{Value, json};

async fn engine(e: &Env) -> Value {
    e.get("/api/engine").await.body
}

async fn versions(e: &Env) -> Value {
    e.get("/api/engine/versions").await.body
}

/// Contrôle du registre dû au prochain `tick`.
async fn recheck(e: &Env) {
    let r = e.send("PUT", "/api/engine/auto-update", json!({"on": true})).await;
    assert_eq!(r.body, json!({"on": true}), "{}", r.text);
}

fn installed(e: &Env, tag: &str) -> bool {
    e.dir.path().join("data/engines").join(tag).exists()
}

/// Variables du faux llama-server pour un flux lent (≈ 2 s).
fn slow_script(dir: &std::path::Path) -> String {
    let p = dir.join("slow.json");
    std::fs::write(&p, json!([{"content": "y".repeat(80)}]).to_string()).unwrap();
    format!("FAKE_CHUNK_DELAY_MS=100 FAKE_CHAT_SCRIPT={}", p.display())
}

/// Requête /v1 lente (≈ 2 s) lancée en fond.
fn slow_request(base: &str, key: &str) -> tokio::task::JoinHandle<String> {
    let (base, key) = (base.to_string(), key.to_string());
    tokio::spawn(async move {
        reqwest::Client::new()
            .post(format!("{base}/v1/responses"))
            .bearer_auth(key)
            .json(&json!({"stream": true, "input": "lent"}))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap()
    })
}

async fn wait_gate_taken(e: &Env) {
    for _ in 0..100 {
        if e.state.serve_gate().try_write().is_err() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("aucune requête en cours");
}

#[tokio::test]
async fn pending_until_idle_then_switch() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = slow_script(tmp.path());
    let e = env_with(&fake).await;
    make_active_preset(&e).await;
    let key = new_key(&e, "t").await;
    let base = serve_tcp(e.router.clone()).await;
    assert_eq!(engine(&e).await["auto_update"], true);

    // Requête en cours : la nouvelle version est téléchargée mais attend.
    publish(&e.reg, "b12500", &fake);
    let slow = slow_request(&base, &key);
    wait_gate_taken(&e).await;
    tick(&e.state).await;
    let v = engine(&e).await;
    assert_eq!(v["pending"]["tag"], "server-cuda-b12500", "{v}");
    assert_eq!(versions(&e).await["active"], "image");
    assert!(installed(&e, "server-cuda-b12500"));
    tick(&e.state).await;
    assert_eq!(versions(&e).await["active"], "image", "porte prise : pas de bascule");
    slow.await.unwrap();

    // Dernière requête trop récente (idle_for = 300 ms) : toujours en attente.
    tick(&e.state).await;
    assert_eq!(versions(&e).await["active"], "image");

    // Au repos : bascule.
    tokio::time::sleep(Duration::from_millis(400)).await;
    tick(&e.state).await;
    let v = versions(&e).await;
    assert_eq!(v["active"], "server-cuda-b12500", "{v}");
    assert_eq!(v["job"]["kind"], "auto_update");
    let v = engine(&e).await;
    assert!(v["pending"].is_null(), "{v}");
    assert_eq!(v["auto_last"]["ok"], true, "{v}");
    assert_eq!(v["state"]["state"], "ready");
}

#[tokio::test]
async fn without_preset_switches_at_once_and_off_does_nothing() {
    let e = env().await;
    let r = e.send("PUT", "/api/engine/auto-update", json!({"on": false})).await;
    assert_eq!(r.body, json!({"on": false}));
    publish(&e.reg, "b12500", "");
    tick(&e.state).await;
    assert!(
        versions(&e).await["latest"].is_null(),
        "réglage off : aucun appel au registre"
    );
    assert_eq!(engine(&e).await["auto_update"], false);
    recheck(&e).await;
    tick(&e.state).await;
    let v = versions(&e).await;
    assert_eq!(v["active"], "server-cuda-b12500", "{v}");
    assert_eq!(engine(&e).await["auto_last"]["ok"], true);
}

#[tokio::test]
async fn env_override_disables_auto_update() {
    let e = env_cfg("", |c| c.llama_bin = Some(c.image_bin.clone())).await;
    publish(&e.reg, "b12500", "");
    tick(&e.state).await;
    assert!(
        versions(&e).await["latest"].is_null(),
        "ENGINEX_LLAMA_BIN : aucun appel au registre"
    );
    assert_eq!(engine(&e).await["auto_update"], false);
    let r = e.send("PUT", "/api/engine/auto-update", json!({"on": true})).await;
    assert_eq!(r.status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn failing_version_is_skipped_until_manual_install() {
    let e = env().await;
    make_active_preset(&e).await;
    publish(&e.reg, "b12600", "FAKE_OOM=1");
    tick(&e.state).await;
    let v = engine(&e).await;
    assert_eq!(v["skipped"], json!(["server-cuda-b12600"]), "{v}");
    assert_eq!(v["auto_last"]["ok"], false);
    assert!(
        v["auto_last"]["message"].as_str().unwrap().contains("load_failed"),
        "{v}"
    );
    assert!(v["pending"].is_null());
    assert_eq!(versions(&e).await["active"], "image");
    e.until("/api/engine", |v| v["state"]["state"] == "ready").await;

    // Pas retentée.
    recheck(&e).await;
    tick(&e.state).await;
    assert!(engine(&e).await["pending"].is_null());
    assert_eq!(versions(&e).await["job"]["tag"], "server-cuda-b12600");
    assert_eq!(versions(&e).await["job"]["kind"], "auto_update");

    // Installation à la main : retirée de la liste (même si elle échoue encore).
    let r = e.send("POST", "/api/engine/versions/install", json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    e.until("/api/engine/versions", |v| {
        v["job"]["kind"] == "install" && v["job"]["finished"] == true
    })
    .await;
    assert_eq!(engine(&e).await["skipped"], json!([]));
}

#[tokio::test]
async fn tools_regression_rolls_back_auto_and_manual() {
    let e = env().await;
    make_active_preset(&e).await;
    assert_eq!(engine(&e).await["tools_ok"], true);

    // Automatique.
    publish(&e.reg, "b12700", "FAKE_NO_TOOLS=1");
    tick(&e.state).await;
    let v = engine(&e).await;
    assert_eq!(v["skipped"], json!(["server-cuda-b12700"]), "{v}");
    assert!(
        v["auto_last"]["message"].as_str().unwrap().contains("tools_broken"),
        "{v}"
    );
    assert_eq!(versions(&e).await["active"], "image");
    let v = e
        .until("/api/engine", |v| {
            v["state"]["state"] == "ready" && v["tools_ok"] == true
        })
        .await;
    assert_eq!(v["origin"], "image");

    // Manuelle.
    publish(&e.reg, "b12800", "FAKE_NO_TOOLS=1");
    e.send("POST", "/api/engine/versions/check", json!({})).await;
    let r = e.send("POST", "/api/engine/versions/install", json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let v = e
        .until("/api/engine/versions", |v| {
            v["job"]["kind"] == "install" && v["job"]["finished"] == true
        })
        .await;
    assert_eq!(v["job"]["step"], "failed", "{v}");
    assert!(v["job"]["error"].as_str().unwrap().contains("tools_broken"), "{v}");
    assert_eq!(v["active"], "image");
    let skipped = engine(&e).await["skipped"].clone();
    assert!(
        skipped.as_array().unwrap().contains(&json!("server-cuda-b12800")),
        "{skipped}"
    );
}

#[tokio::test]
async fn newer_build_replaces_pending_and_manual_switch_keeps_it() {
    let tmp = tempfile::tempdir().unwrap();
    let fake = slow_script(tmp.path());
    let e = env_with(&fake).await;
    make_active_preset(&e).await;
    let key = new_key(&e, "t").await;
    let base = serve_tcp(e.router.clone()).await;

    let slow = slow_request(&base, &key);
    wait_gate_taken(&e).await;
    publish(&e.reg, "b12500", "");
    tick(&e.state).await;
    assert_eq!(engine(&e).await["pending"]["tag"], "server-cuda-b12500");
    publish(&e.reg, "b12510", "");
    recheck(&e).await;
    tick(&e.state).await;
    assert_eq!(engine(&e).await["pending"]["tag"], "server-cuda-b12510");
    assert!(
        !installed(&e, "server-cuda-b12500"),
        "ancienne version en attente supprimée"
    );
    assert!(installed(&e, "server-cuda-b12510"));
    slow.await.unwrap();

    // Bascule manuelle vers une troisième : la version en attente reste.
    publish(&e.reg, "b12520", "");
    e.send("POST", "/api/engine/versions/check", json!({})).await;
    let r = e.send("POST", "/api/engine/versions/install", json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let v = e
        .until("/api/engine/versions", |v| {
            v["job"]["kind"] == "install" && v["job"]["finished"] == true
        })
        .await;
    assert_eq!(v["active"], "server-cuda-b12520", "{v}");
    assert!(
        installed(&e, "server-cuda-b12510"),
        "version en attente gardée par cleanup"
    );
}
