//! Agents (étape 7c) : réglages, départ dans une session neuve avec le faux
//! app-server, budget, déclencheurs (fil, webhook, autre agent), garde-fous
//! des chaînes. `cargo build -p enginex-fake-llama` d'abord.

mod common;

use std::time::Duration;

use axum::http::StatusCode;
use common::*;
use serde_json::{Value, json};

async fn create(e: &Env, body: Value) -> i64 {
    let r = e.send("POST", "/api/agents", body).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    r.body["id"].as_i64().unwrap()
}

/// Attend `n` départs dont le dernier est fini, et le rend.
async fn last_run(e: &Env, id: i64, n: usize) -> Value {
    let runs = e
        .until(&format!("/api/agents/{id}/runs"), |v| {
            let a = v.as_array().unwrap();
            a.len() >= n && a[0]["status"] != "running"
        })
        .await;
    runs[0].clone()
}

async fn runs(e: &Env, id: i64) -> Vec<Value> {
    e.get(&format!("/api/agents/{id}/runs"))
        .await
        .body
        .as_array()
        .unwrap()
        .clone()
}

async fn hook(e: &Env, path: &str, body: &str) -> u16 {
    let base = serve_tcp(e.router.clone()).await;
    reqwest::Client::new()
        .post(format!("{base}{path}"))
        .body(body.to_string())
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

#[tokio::test]
async fn settings_are_checked() {
    let e = env().await;
    let topic = inbox(&e).await;
    let ok = json!({"name": "Veille", "topic_id": topic, "role": "Surveille les ventes."});
    for (patch, why) in [
        (json!({"name": " "}), "nom vide"),
        (json!({"role": ""}), "rôle vide"),
        (json!({"approval": "jamais"}), "approbation"),
        (json!({"schedule": "* * *"}), "horaire"),
        (json!({"tz": "Mars/Olympus"}), "fuseau"),
        (json!({"timeout_s": 5}), "délai"),
        (json!({"budget_gpu_min": -1}), "budget"),
        (json!({"feed_filter": {"min_level": "énorme"}}), "niveau du filtre"),
        (json!({"topic_id": 9999}), "topic"),
        (json!({"preset_id": 9999}), "preset"),
    ] {
        let mut b = ok.clone();
        for (k, v) in patch.as_object().unwrap() {
            b[k] = v.clone();
        }
        let r = e.send("POST", "/api/agents", b).await;
        assert!(r.status.is_client_error(), "{why} : {}", r.text);
    }
    let mut b = ok.clone();
    b["schedule"] = json!("0 9 * * 1-5");
    let id = create(&e, b).await;
    let r = e.send("POST", "/api/agents", ok.clone()).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "nom déjà pris : {}", r.text);
    let list = e.get("/api/agents").await.body;
    assert_eq!(list[0]["name"], "Veille");
    assert!(list[0]["next_run"].as_i64().unwrap() > enginex_server::state::now());
    assert_eq!(list[0]["today"]["tokens"], 0);
    // Coupé : plus de prochaine exécution ; même nom gardé à la modification.
    let mut b = ok.clone();
    b["enabled"] = json!(false);
    b["schedule"] = json!("every:30");
    let r = e.send("PUT", &format!("/api/agents/{id}"), b).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert!(e.get("/api/agents").await.body[0]["next_run"].is_null());
    let r = e.send("DELETE", &format!("/api/agents/{id}"), json!({})).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(e.get("/api/agents").await.body.as_array().unwrap().is_empty());
}

#[tokio::test]
async fn run_opens_a_new_session_and_counts_tokens() {
    let (e, fake) = env_sessions(&script_with("text", |_| {})).await;
    make_active_preset(&e).await;
    let topic = inbox(&e).await;
    let id = create(
        &e,
        json!({"name": "Rapport", "topic_id": topic, "role": "Écris le rapport du matin.", "notify": true}),
    )
    .await;
    let r = e
        .send(
            "POST",
            &format!("/api/agents/{id}/run"),
            json!({"context": "Pense aux ventes d'hier."}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    // Déjà en cours : refusé, et tracé.
    let again = e.send("POST", &format!("/api/agents/{id}/run"), json!({})).await;
    assert_eq!(again.status, StatusCode::SERVICE_UNAVAILABLE, "{}", again.text);
    let run = e
        .until(&format!("/api/agents/{id}/runs"), |v| {
            v.as_array()
                .unwrap()
                .iter()
                .any(|r| r["status"] == "ok" || r["status"] == "failed")
        })
        .await
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["status"] != "skipped")
        .cloned()
        .unwrap();
    assert_eq!(run["status"], "ok", "{run}");
    assert_eq!(run["trigger"], "manual");
    assert_eq!(run["output"], "Bonjour, je suis prêt.");
    assert_eq!(run["tokens"], 120);
    let sid = run["session_id"].as_str().unwrap();
    let s = e.get(&format!("/api/sessions/{sid}")).await.body;
    assert_eq!(s["session"]["title"], "Agent : Rapport");
    assert_eq!(s["turns"][0]["origin"], "agent", "{s}");
    let start = &fake.received("turn/start")[0]["params"];
    let text = start["input"][0]["text"].as_str().unwrap();
    assert!(text.contains("Tu es l'agent « Rapport »"), "{text}");
    assert!(text.contains("Écris le rapport du matin.") && text.contains("Pense aux ventes d'hier."));
    let a = &e.get("/api/agents").await.body[0];
    assert_eq!(a["today"]["tokens"], 120);
    assert_eq!(a["last"]["status"], "ok");
    let f = e
        .until("/api/feed", |v| {
            v["items"].as_array().unwrap().iter().any(|i| i["source"] == "agent")
        })
        .await;
    let n = f["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["source"] == "agent")
        .unwrap();
    assert_eq!(n["title"], "Agent « Rapport » : fini");
    assert_eq!(n["link"], format!("#sessions/{sid}"));
    // Le tour d'agent ne poste pas en plus une note « réponse prête ».
    assert!(!f.to_string().contains("réponse prête"));
}

#[tokio::test]
async fn budget_stops_departures_until_midnight() {
    let e = env().await;
    let topic = inbox(&e).await;
    let id = create(
        &e,
        json!({"name": "Gourmand", "topic_id": topic, "role": "x", "budget_tokens": 1000}),
    )
    .await;
    sqlx::query(
        "INSERT INTO agent_runs (agent_id, trigger, status, started_at, ended_at, tokens) VALUES (?, 'manual', 'ok', ?, ?, 1500)",
    )
    .bind(id)
    .bind(enginex_server::state::now())
    .bind(enginex_server::state::now())
    .execute(e.state.db())
    .await
    .unwrap();
    for _ in 0..2 {
        let r = e.send("POST", &format!("/api/agents/{id}/run"), json!({})).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
        assert!(r.text.contains("budget"), "{}", r.text);
    }
    let rs = runs(&e, id).await;
    assert_eq!(rs[0]["status"], "skipped");
    assert_eq!(rs[1]["status"], "skipped");
    // Une seule notification par jour.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let f = e.get("/api/feed").await.body;
    let n: Vec<&Value> = f["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["title"] == "Agent « Gourmand » : budget atteint")
        .collect();
    assert_eq!(n.len(), 1, "{f}");
}

#[tokio::test]
async fn feed_webhook_and_agent_triggers() {
    // Pas de moteur : chaque départ échoue vite, mais il est tracé.
    let e = env().await;
    let topic = inbox(&e).await;
    let ventes = create(
        &e,
        json!({"name": "Ventes", "topic_id": topic, "role": "Traite les ventes.",
               "feed_filter": {"sources": ["webhook"], "contains": "vente"}}),
    )
    .await;
    let suivi = create(
        &e,
        json!({"name": "Suivi", "topic_id": topic, "role": "Suit les échecs des agents.",
               "feed_filter": {"sources": ["agent"], "min_level": "error"}}),
    )
    .await;
    let ci = create(&e, json!({"name": "CI", "topic_id": topic, "role": "Regarde la CI."})).await;

    // Webhook libre : le fil lance « Ventes », dont l'échec lance « Suivi ».
    let r = e.send("POST", "/api/webhooks", json!({"name": "boutique"})).await;
    let shop = r.body["path"].as_str().unwrap().to_string();
    assert_eq!(hook(&e, &shop, "Nouvelle vente\n12 €").await, 202);
    let run = last_run(&e, ventes, 1).await;
    assert_eq!(run["trigger"], "feed");
    assert_eq!(run["depth"], 0);
    assert_eq!(run["status"], "failed");
    assert!(
        run["context"].as_str().unwrap().contains("boutique : Nouvelle vente"),
        "{run}"
    );
    let s = last_run(&e, suivi, 1).await;
    assert_eq!(s["trigger"], "feed");
    assert_eq!(s["depth"], 1);
    assert!(
        s["context"].as_str().unwrap().contains("Agent « Ventes » : en échec"),
        "{s}"
    );
    // « Suivi » ne se relance pas sur son propre échec ; une minute entre deux
    // départs par le fil.
    assert_eq!(hook(&e, &shop, "Autre vente").await, 202);
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(runs(&e, suivi).await.len(), 1);
    assert_eq!(runs(&e, ventes).await.len(), 1);

    // Webhook lié à « CI » : départ direct, pas de 2e départ par le fil.
    let r = e
        .send("POST", "/api/webhooks", json!({"name": "forge", "agent_id": ci}))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let forge = r.body["path"].as_str().unwrap().to_string();
    let forge_row = |v: Value| {
        v.as_array()
            .unwrap()
            .iter()
            .find(|h| h["name"] == "forge")
            .cloned()
            .unwrap()
    };
    assert_eq!(forge_row(e.get("/api/webhooks").await.body)["agent_id"], ci);
    assert_eq!(hook(&e, &forge, "build rouge").await, 202);
    let run = last_run(&e, ci, 1).await;
    assert_eq!(run["trigger"], "webhook");
    assert!(
        run["context"]
            .as_str()
            .unwrap()
            .starts_with("Webhook « forge » : build rouge")
    );
    let wid = r.body["id"].as_i64().unwrap();
    let r = e
        .send("PATCH", &format!("/api/webhooks/{wid}"), json!({"agent_id": 9999}))
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = e
        .send("PATCH", &format!("/api/webhooks/{wid}"), json!({"agent_id": null}))
        .await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(forge_row(e.get("/api/webhooks").await.body)["agent_id"].is_null());

    // agent_send depuis une session, puis depuis un départ d'agent.
    let sid = new_session(&e, "auto").await;
    let key = sandbox_key(&e, &sid).await;
    let (t, err) = mcp_tool(&e, &key, "agent_list", json!({})).await;
    assert!(
        !err && t.contains("- CI : Regarde la CI.") && t.contains("- Ventes : Traite les ventes."),
        "{t}"
    );
    let (t, err) = mcp_tool(
        &e,
        &key,
        "agent_send",
        json!({"name": "ci", "message": "relance le build"}),
    )
    .await;
    assert!(!err, "{t}");
    let run = last_run(&e, ci, 2).await;
    assert_eq!(run["trigger"], "session");
    assert!(run["context"].as_str().unwrap().contains("relance le build"));
    let (t, err) = mcp_tool(&e, &key, "agent_send", json!({"name": "Personne", "message": "x"})).await;
    assert!(err && t.contains("agent inconnu"), "{t}");

    let ci_run = runs(&e, ci).await[0]["session_id"].as_str().unwrap().to_string();
    let ci_key = sandbox_key(&e, &ci_run).await;
    let (t, err) = mcp_tool(&e, &ci_key, "agent_send", json!({"name": "CI", "message": "x"})).await;
    assert!(err && t.contains("lui-même"), "{t}");
    let (t, err) = mcp_tool(
        &e,
        &ci_key,
        "agent_send",
        json!({"name": "Ventes", "message": "compte"}),
    )
    .await;
    assert!(!err, "{t}");
    let run = last_run(&e, ventes, 2).await;
    assert_eq!(run["trigger"], "agent");
    assert_eq!(run["depth"], 1);
    assert!(run["context"].as_str().unwrap().contains("confié par l'agent « CI »"));
    sqlx::query("UPDATE agent_runs SET depth = 3 WHERE session_id = ?")
        .bind(&ci_run)
        .execute(e.state.db())
        .await
        .unwrap();
    let (t, err) = mcp_tool(&e, &ci_key, "agent_send", json!({"name": "Suivi", "message": "x"})).await;
    assert!(err && t.contains("trop longue"), "{t}");
}

#[tokio::test]
async fn scheduler_runs_due_agents_once() {
    let e = env().await;
    let topic = inbox(&e).await;
    let id = create(
        &e,
        json!({"name": "Tôt", "topic_id": topic, "role": "x", "schedule": "every:60"}),
    )
    .await;
    sqlx::query("UPDATE agents SET next_run = ? WHERE id = ?")
        .bind(enginex_server::state::now() - 3 * 3600)
        .bind(id)
        .execute(e.state.db())
        .await
        .unwrap();
    let run = last_run(&e, id, 1).await;
    assert_eq!(run["trigger"], "schedule");
    assert_eq!(run["status"], "failed");
    assert!(run["output"].as_str().unwrap().contains("preset"), "{run}");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(runs(&e, id).await.len(), 1);
    let next = e.get("/api/agents").await.body[0]["next_run"].as_i64().unwrap();
    let now = enginex_server::state::now();
    assert!((now + 3590..=now + 3600).contains(&next), "{next} vs {now}");
}
