//! Page Tester : un test lancé avec le faux app-server, contrôles à points,
//! verdict, note visuelle, comparaison, suppression. `cargo build -p enginex-fake-llama` d'abord.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::{Value, json};

fn exec(code: i64, out: &str) -> Value {
    json!({"expect": "command/exec", "result": {"exitCode": code, "stdout": out, "stderr": ""}})
}

/// Tour « text » puis les réponses aux `command/exec` de l'audit (dans l'ordre).
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
async fn the_fifteen_tests_are_listed() {
    let e = env().await;
    let r = e.get("/api/bench/cases").await;
    assert_eq!(r.status, StatusCode::OK);
    let list = r.body.as_array().unwrap();
    assert_eq!(list.len(), 15);
    let fam = |f: &str| list.iter().filter(|c| c["family"] == f).count();
    assert_eq!((fam("visual"), fam("code"), fam("tools")), (7, 5, 3));
    assert!(list.iter().all(|c| c["points_max"] == 20), "barème sur 20 partout");
    assert!(
        list.iter()
            .all(|c| c["checks"].as_array().unwrap().iter().any(|k| k["essential"] == true))
    );
}

#[tokio::test]
async fn start_is_refused_for_bad_input_or_a_running_run() {
    let (e, _fake) = env_sessions(&script(&[])).await;
    // Cas inconnu.
    let r = e.send("POST", "/api/bench/runs", json!({"cases": ["nope"]})).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
    // Aucun preset actif.
    let r = e
        .send("POST", "/api/bench/runs", json!({"cases": ["calculator"]}))
        .await;
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
    let r = e
        .send("POST", "/api/bench/runs", json!({"cases": ["calculator"]}))
        .await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.text);
}

#[tokio::test]
async fn stop_marks_a_running_run_stopped() {
    let e = env().await;
    sqlx::query("INSERT INTO bench_runs (preset_id, status, started_at) VALUES (1, 'running', 1)")
        .execute(e.state.db())
        .await
        .unwrap();
    sqlx::query("INSERT INTO bench_cases (run_id, case_key, status) VALUES (1, 'calculator', 'pending')")
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
async fn a_test_runs_and_is_checked_with_points() {
    // bugfix : rm des tests cachés, tests fournis OK, tests cachés 7/10, rm final.
    let (e, fake) = env_sessions(&script(&[
        (0, ""),
        (0, "OK"),
        (1, "KO arrondi\nBENCH_SCORE 7/10\n"),
        (0, ""),
    ]))
    .await;
    make_active_preset(&e).await;
    // Une mémoire globale existe : le test ne doit pas la recevoir.
    let mem = e.dir.path().join("data/workspace/memory/memory.md");
    std::fs::create_dir_all(mem.parent().unwrap()).unwrap();
    std::fs::write(&mem, "- SOLUTION-SECRETE\n").unwrap();
    let r = e.send("POST", "/api/bench/runs", json!({"cases": ["bugfix"]})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let id = r.body["id"].as_i64().unwrap();
    let run = run_done(&e, id).await;
    assert_eq!(run["status"], "done", "{run}");
    let c = &run["cases"][0];
    assert_eq!(c["status"], "done", "{c}");
    assert_eq!(c["family"], "code");
    let (st, fi) = (c["started_at"].as_i64().unwrap(), c["finished_at"].as_i64().unwrap());
    assert!(fi >= st, "{c}");
    // 6 (tests fournis) + 7 (prorata 7/10) + 2 (tests intacts) + 0 (jamais lancé les tests).
    assert_eq!(c["points"].as_f64(), Some(15.0), "{c}");
    assert_eq!(c["points_max"], 20);
    assert_eq!(c["verdict"], "partial", "{c}");
    let checks = c["audit"]["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 4);
    assert_eq!(checks[1]["earned"].as_f64(), Some(7.0));
    assert!(checks[1]["proof"].as_str().unwrap().contains("KO arrondi"));
    assert_eq!(checks[2]["ok"], true, "tests fournis intacts : {}", checks[2]);
    assert_eq!(checks[3]["ok"], false, "{}", checks[3]);
    assert!(c["audit"]["metrics"]["turns"].as_i64().unwrap() >= 1, "{c}");
    // Les tests cachés sont écrits seulement pour l'audit, puis retirés par la sandbox.
    let execs = fake.received("command/exec");
    assert_eq!(execs.len(), 4, "{execs:?}");
    let cmd = |i: usize| execs[i]["params"]["command"].to_string();
    assert!(cmd(0).contains("rm -rf .bench-hidden") && cmd(3).contains("rm -rf .bench-hidden"));
    assert!(cmd(2).contains("h_cart.py"));
    // Session : origine bench, un seul tour (ni correction ni essai d'app), sans secrets.
    let sid = c["session_id"].as_str().unwrap();
    let s = e.get(&format!("/api/sessions/{sid}")).await.body;
    assert_eq!(s["turns"].as_array().unwrap().len(), 1, "{s}");
    assert_eq!(s["turns"][0]["origin"], "bench", "{s}");
    assert!(enginex_server::bench::is_bench_session(&e.state, sid).await);
    let other = new_session(&e, "auto").await;
    assert!(!enginex_server::bench::is_bench_session(&e.state, &other).await);
    let text = fake.received("turn/start")[0]["params"]["input"][0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(text.contains("cart.py"), "{text}");
    let instr = fake.received("thread/start")[0]["params"]["developerInstructions"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        !instr.contains("SOLUTION-SECRETE") && !instr.contains("EngineX garde des notes"),
        "{instr}"
    );
    assert!(instr.contains("Session de test"), "{instr}");
    assert!(
        !text.contains("SOLUTION-SECRETE") && !text.contains("<memoire_enginex>"),
        "{text}"
    );
    // Test de code : pas de note visuelle, commentaire accepté.
    let cid = c["id"].as_i64().unwrap();
    let url = format!("/api/bench/runs/{id}/cases/{cid}/rate");
    assert_eq!(
        e.send("POST", &url, json!({"visual": 4})).await.status,
        StatusCode::BAD_REQUEST
    );
    let ok = e.send("POST", &url, json!({"comment": "propre"})).await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text);
    assert_eq!(
        e.get(&format!("/api/bench/runs/{id}")).await.body["cases"][0]["comment"],
        "propre"
    );
    // Résumé du run et comparaison.
    let runs = e.get("/api/bench/runs").await.body;
    let sum = &runs[0]["summary"];
    assert_eq!(
        (sum["passed"].as_i64(), sum["partial"].as_i64()),
        (Some(0), Some(1)),
        "{sum}"
    );
    assert_eq!(sum["result"].as_f64(), Some(75.0), "{sum}");
    let cmp = e.get("/api/bench/compare").await.body;
    assert_eq!(cmp[0]["runs"], 1);
    assert_eq!(cmp[0]["cases"][0]["key"], "bugfix");
    assert_eq!(cmp[0]["cases"][0]["result"].as_f64(), Some(75.0));
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
async fn a_visual_test_takes_a_rating_from_1_to_5() {
    let e = env().await;
    sqlx::query("INSERT INTO bench_runs (preset_id, status, started_at) VALUES (1, 'done', 1)")
        .execute(e.state.db())
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO bench_cases (run_id, case_key, status, points, points_max, verdict, audited_at) \
         VALUES (1, 'calculator', 'done', 20, 20, 'passed', 2)",
    )
    .execute(e.state.db())
    .await
    .unwrap();
    let url = "/api/bench/runs/1/cases/1/rate";
    assert_eq!(
        e.send("POST", url, json!({"visual": 6})).await.status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        e.send("POST", url, json!({"visual": 0})).await.status,
        StatusCode::BAD_REQUEST
    );
    let r = e.send("POST", url, json!({"visual": 4, "comment": "joli"})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    // Un commentaire seul garde la note.
    assert_eq!(
        e.send("POST", url, json!({"comment": "très joli"})).await.status,
        StatusCode::OK
    );
    let run = e.get("/api/bench/runs/1").await.body;
    assert_eq!(run["cases"][0]["visual"], 4);
    assert_eq!(run["summary"]["visual"].as_f64(), Some(4.0));
    assert_eq!(run["summary"]["to_rate"], 0);
    assert_eq!(run["summary"]["passed"], 1);
    // Pas de capture : 404 ; nom inconnu : 404.
    assert_eq!(
        e.get("/api/bench/cases/1/shots/desktop").await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        e.get("/api/bench/cases/1/shots/..%2Fx").await.status,
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
    sqlx::query("INSERT INTO bench_cases (run_id, case_key, status) VALUES (1, 'calculator', 'running')")
        .execute(e.state.db())
        .await
        .unwrap();
    let r = e
        .send("POST", "/api/bench/runs/1/cases/1/rate", json!({"visual": 3}))
        .await;
    assert_eq!(r.status, StatusCode::CONFLICT, "{}", r.text);
}
