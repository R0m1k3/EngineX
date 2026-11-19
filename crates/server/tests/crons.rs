//! Crons : réglages, exécution d'une consigne (tour Codex) et d'une commande
//! (`command/exec`) avec le faux app-server, planificateur, historique.
//! `cargo build -p enginex-fake-llama` d'abord.

mod common;

use std::time::Duration;

use axum::http::StatusCode;
use common::*;
use serde_json::{Value, json};

fn exec_result(code: i64, out: &str) -> Value {
    json!({"exitCode": code, "stdout": out, "stderr": ""})
}

/// Démarrage de sandbox du script « text » (jusqu'au premier tour exclu).
fn sandbox_only() -> Vec<Value> {
    let steps = script_with("text", |_| {});
    let start = step_pos(&steps, 0, is_expect("turn/start"));
    // Pas de tour : la session d'un cron shell n'est jamais renommée.
    steps[..start]
        .iter()
        .filter(|s| s["expect"] != "thread/name/set")
        .cloned()
        .collect()
}

async fn create(e: &Env, body: Value) -> i64 {
    let r = e.send("POST", "/api/crons", body).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    r.body["id"].as_i64().unwrap()
}

/// Attend la fin de la dernière exécution et la rend.
async fn last_run(e: &Env, id: i64, n: usize) -> Value {
    let runs = e
        .until(&format!("/api/crons/{id}/runs"), |v| {
            let a = v.as_array().unwrap();
            a.len() >= n && a[0]["status"] != "running"
        })
        .await;
    runs[0].clone()
}

#[tokio::test]
async fn settings_are_checked() {
    let e = env().await;
    let topic = inbox(&e).await;
    let ok = json!({"name": "Rapport", "topic_id": topic, "kind": "prompt", "body": "Fais le point",
                    "schedule": "0 9 * * 1-5"});
    for (field, v, why) in [
        ("schedule", json!("tous les jours"), "5 champs"),
        ("schedule", json!("every:0"), "intervalle"),
        ("tz", json!("Lune/Mer"), "fuseau"),
        ("kind", json!("python"), "prompt ou shell"),
        ("name", json!(""), "nom"),
        ("timeout_s", json!(3), "délai"),
    ] {
        let mut b = ok.clone();
        b[field] = v;
        let r = e.send("POST", "/api/crons", b).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{field}");
        assert!(
            r.body["detail"].as_str().unwrap_or("").contains(why),
            "{field} : {}",
            r.body
        );
    }
    let id = create(&e, ok.clone()).await;
    let list = e.get("/api/crons").await.body;
    assert_eq!(list[0]["name"], "Rapport");
    assert_eq!(list[0]["tz"], "Europe/Paris");
    assert_eq!(list[0]["topic"], "general / inbox");
    assert!(list[0]["next_run"].as_i64().unwrap() > enginex_server::state::now());
    // Désactivé : plus de prochaine exécution.
    let mut b = ok.clone();
    b["enabled"] = json!(false);
    let r = e.send("PUT", &format!("/api/crons/{id}"), b).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert!(e.get("/api/crons").await.body[0]["next_run"].is_null());
    // Aperçu : 3 dates croissantes.
    let p = e
        .send("POST", "/api/crons/preview", json!({"schedule": "every:30"}))
        .await
        .body;
    let n: Vec<i64> = p["next"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_i64().unwrap())
        .collect();
    assert_eq!((n[1] - n[0], n[2] - n[1]), (1800, 1800));
    // Topic avec un cron : non supprimable.
    let r = e.send("DELETE", &format!("/api/topics/{topic}"), json!({})).await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    let r = e.send("DELETE", &format!("/api/crons/{id}"), json!({})).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(e.get("/api/crons").await.body.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn prompt_cron_runs_a_turn_in_its_own_session() {
    let (e, fake) = env_sessions(&script_with("text", |_| {})).await;
    make_active_preset(&e).await;
    let topic = inbox(&e).await;
    let id = create(
        &e,
        json!({"name": "Salut", "topic_id": topic, "kind": "prompt", "body": "Dis bonjour", "schedule": "every:60"}),
    )
    .await;
    let r = e.send("POST", &format!("/api/crons/{id}/run"), json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    // Déjà en cours : refusé.
    let again = e.send("POST", &format!("/api/crons/{id}/run"), json!({})).await;
    assert_eq!(again.status, StatusCode::SERVICE_UNAVAILABLE, "{}", again.text);
    let run = last_run(&e, id, 1).await;
    assert_eq!(run["status"], "ok", "{run}");
    assert_eq!(run["trigger"], "manual");
    assert_eq!(run["output"], "Bonjour, je suis prêt.");
    let sid = run["session_id"].as_str().unwrap();
    let s = e.get(&format!("/api/sessions/{sid}")).await.body;
    assert_eq!(s["session"]["title"], "Cron : Salut");
    assert_eq!(s["turns"][0]["origin"], "cron", "{s}");
    let start = &fake.received("turn/start")[0]["params"];
    assert_eq!(start["input"][0]["text"], "Dis bonjour");
    assert_eq!(e.get("/api/crons").await.body[0]["session_id"], sid);
}

#[tokio::test]
async fn shell_cron_runs_a_command_in_the_sandbox() {
    let mut script = sandbox_only();
    script.push(json!({"expect": "command/exec", "result": exec_result(0, "bonjour\n")}));
    script.push(json!({"expect": "command/exec", "result": exec_result(2, "raté\n")}));
    let (e, fake) = env_sessions(&script).await;
    make_active_preset(&e).await;
    let topic = inbox(&e).await;
    let id = create(
        &e,
        json!({"name": "Disque", "topic_id": topic, "kind": "shell", "body": "echo bonjour",
               "schedule": "every:5", "timeout_s": 60}),
    )
    .await;
    e.send("POST", &format!("/api/crons/{id}/run"), json!({})).await;
    let run = last_run(&e, id, 1).await;
    assert_eq!(
        (run["status"].as_str(), run["output"].as_str()),
        (Some("ok"), Some("bonjour")),
        "{run}"
    );
    let exec = &fake.received("command/exec")[0]["params"];
    assert_eq!(exec["command"], json!(["bash", "-lc", "echo bonjour"]));
    assert_eq!(exec["timeoutMs"], 60_000);
    // Même session, sandbox réutilisée ; échec rapporté avec le code.
    e.send("POST", &format!("/api/crons/{id}/run"), json!({})).await;
    let run2 = last_run(&e, id, 2).await;
    assert_eq!(run2["status"], "failed");
    assert!(
        run2["output"].as_str().unwrap().ends_with("(code de sortie 2)"),
        "{run2}"
    );
    assert_eq!(run2["session_id"], run["session_id"]);
    assert!(fake.unexpected().is_empty(), "{:?}", fake.unexpected());
}

#[tokio::test]
async fn scheduler_runs_due_crons_once() {
    let e = env().await;
    let topic = inbox(&e).await;
    // Pas de moteur : la consigne échoue, mais l'exécution est tracée.
    let id = create(
        &e,
        json!({"name": "Tôt", "topic_id": topic, "kind": "prompt", "body": "x", "schedule": "every:60"}),
    )
    .await;
    let past = enginex_server::state::now() - 3 * 3600;
    sqlx::query("UPDATE crons SET next_run = ? WHERE id = ?")
        .bind(past)
        .bind(id)
        .execute(e.state.db())
        .await
        .unwrap();
    let run = last_run(&e, id, 1).await;
    assert_eq!(run["trigger"], "schedule");
    assert_eq!(run["status"], "failed");
    assert!(run["output"].as_str().unwrap().contains("preset"), "{run}");
    // Trois heures de retard : une seule exécution, la suivante dans 1 h.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let runs = e.get(&format!("/api/crons/{id}/runs")).await.body;
    assert_eq!(runs.as_array().unwrap().len(), 1);
    let next = e.get("/api/crons").await.body[0]["next_run"].as_i64().unwrap();
    let now = enginex_server::state::now();
    assert!((now + 3590..=now + 3600).contains(&next), "{next} vs {now}");
}
