//! Page Tester : un cas lancé avec le faux app-server, audit des critères,
//! note, comparaison, suppression. `cargo build -p enginex-fake-llama` d'abord.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::{Value, json};

fn exec(code: i64, out: &str) -> Value {
    json!({"expect": "command/exec", "result": {"exitCode": code, "stdout": out, "stderr": ""}})
}

/// Tour « text » puis les résultats des critères du cas (dans l'ordre).
fn script(execs: &[(i64, &str)]) -> Vec<Value> {
    script_with("text", |s| {
        for (c, o) in execs {
            s.push(exec(*c, o));
        }
    })
}

async fn run_done(e: &Env, id: i64) -> Value {
    e.until(&format!("/api/bench/runs/{id}"), |v| v["status"] != "running")
        .await
}

#[tokio::test]
async fn the_ten_cases_are_listed() {
    let e = env().await;
    let r = e.get("/api/bench/cases").await;
    assert_eq!(r.status, StatusCode::OK);
    let list = r.body.as_array().unwrap();
    assert_eq!(list.len(), 10);
    assert!(list.iter().all(|c| c["criteria_count"].as_i64().unwrap() >= 3));
}

#[tokio::test]
async fn start_is_refused_for_bad_input_or_a_running_run() {
    let (e, _fake) = env_sessions(&script(&[])).await;
    // Cas inconnu.
    let r = e.send("POST", "/api/bench/runs", json!({"cases": ["nope"]})).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
    // Aucun preset actif.
    let r = e.send("POST", "/api/bench/runs", json!({"cases": ["counter"]})).await;
    assert!(
        r.status.is_client_error() || r.status == StatusCode::SERVICE_UNAVAILABLE,
        "{}",
        r.text
    );
    make_active_preset(&e).await;
    // Un run déjà actif : 409.
    sqlx::query("INSERT INTO bench_runs (preset_id, status, started_at) VALUES (1, 'running', 1)")
        .execute(e.state.db())
        .await
        .unwrap();
    let r = e.send("POST", "/api/bench/runs", json!({"cases": ["counter"]})).await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.text);
}

#[tokio::test]
async fn stop_marks_a_running_run_stopped() {
    let e = env().await;
    sqlx::query("INSERT INTO bench_runs (preset_id, status, started_at) VALUES (1, 'running', 1)")
        .execute(e.state.db())
        .await
        .unwrap();
    sqlx::query("INSERT INTO bench_cases (run_id, case_key, status) VALUES (1, 'counter', 'pending')")
        .execute(e.state.db())
        .await
        .unwrap();
    let r = e.send("POST", "/api/bench/runs/1/stop", json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let run = e.get("/api/bench/runs/1").await.body;
    assert_eq!(run["status"], "stopped");
    // Le cas jamais lancé ne reste pas « en attente ».
    assert_eq!(run["cases"][0]["status"], "failed");
    let again = e.send("POST", "/api/bench/runs/1/stop", json!({})).await;
    assert_eq!(again.status, StatusCode::NOT_FOUND);
    // Un run arrêté se supprime ; un run actif non.
    assert_eq!(
        e.send("DELETE", "/api/bench/runs/1", json!({})).await.status,
        StatusCode::OK
    );
    assert_eq!(e.get("/api/bench/runs/1").await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_case_runs_is_audited_then_rated() {
    // static-page : 3 critères ; le 3e échoue.
    let (e, fake) = env_sessions(&script(&[(0, "ok1"), (0, "ok2"), (1, "css trop court")])).await;
    make_active_preset(&e).await;
    let r = e
        .send("POST", "/api/bench/runs", json!({"cases": ["static-page"]}))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let id = r.body["id"].as_i64().unwrap();
    // Pas de second run pendant celui-ci (ou déjà fini : jamais deux actifs).
    let run = run_done(&e, id).await;
    assert_eq!(run["status"], "done", "{run}");
    let c = &run["cases"][0];
    assert_eq!(c["status"], "done", "{c}");
    let auto = c["auto_score"].as_f64().unwrap();
    assert!((auto - 200.0 / 3.0).abs() < 0.01, "{auto}");
    let crit = c["audit"]["criteria"].as_array().unwrap();
    assert_eq!(crit.len(), 3);
    assert_eq!(crit[2]["ok"], false);
    assert!(crit[2]["proof"].as_str().unwrap().contains("css trop court"));
    assert!(c["audit"]["metrics"]["turns"].as_i64().unwrap() >= 1, "{c}");
    // Session : origine bench, un seul tour (ni correction ni essai d'app), sans secrets.
    let sid = c["session_id"].as_str().unwrap();
    let s = e.get(&format!("/api/sessions/{sid}")).await.body;
    assert_eq!(s["turns"].as_array().unwrap().len(), 1, "{s}");
    assert_eq!(s["turns"][0]["origin"], "bench", "{s}");
    assert!(enginex_server::bench::is_bench_session(&e.state, sid).await);
    let other = new_session(&e, "auto").await;
    assert!(!enginex_server::bench::is_bench_session(&e.state, &other).await);
    // Le modèle a reçu la consigne du cas.
    let text = fake.received("turn/start")[0]["params"]["input"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(text.contains("Le Fournil"), "{text}");
    // Note : hors bornes refusée, puis acceptée ; finale = (auto + manuelle) / 2.
    let cid = c["id"].as_i64().unwrap();
    let url = format!("/api/bench/runs/{id}/cases/{cid}/rate");
    let bad = e
        .send(
            "POST",
            &url,
            json!({"fonctionne": 6, "qualite": 3, "rendu": 3, "consigne": 3}),
        )
        .await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST, "{}", bad.text);
    let ok = e
        .send(
            "POST",
            &url,
            json!({"fonctionne": 5, "qualite": 5, "rendu": 5, "consigne": 5, "comment": "propre"}),
        )
        .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text);
    let got = e.get(&format!("/api/bench/runs/{id}")).await.body["cases"][0].clone();
    assert_eq!(got["manual_score"], 100.0);
    let fin = got["final"].as_f64().unwrap();
    assert!((fin - (auto + 100.0) / 2.0).abs() < 0.01, "{got}");
    assert_eq!(got["comment"], "propre");
    // Liste et comparaison.
    let runs = e.get("/api/bench/runs").await.body;
    assert_eq!(runs[0]["rated"], 1);
    let cmp = e.get("/api/bench/compare").await.body;
    assert_eq!(cmp[0]["runs"], 1);
    assert_eq!(cmp[0]["cases"][0]["key"], "static-page");
    // Supprimer le run supprime sa session.
    assert_eq!(
        e.send("DELETE", &format!("/api/bench/runs/{id}"), json!({}))
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        e.get(&format!("/api/sessions/{sid}")).await.status,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn rating_is_refused_before_the_audit() {
    let e = env().await;
    sqlx::query("INSERT INTO bench_runs (preset_id, status, started_at) VALUES (1, 'done', 1)")
        .execute(e.state.db())
        .await
        .unwrap();
    sqlx::query("INSERT INTO bench_cases (run_id, case_key, status) VALUES (1, 'counter', 'running')")
        .execute(e.state.db())
        .await
        .unwrap();
    let r = e
        .send(
            "POST",
            "/api/bench/runs/1/cases/1/rate",
            json!({"fonctionne": 3, "qualite": 3, "rendu": 3, "consigne": 3}),
        )
        .await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.text);
}
