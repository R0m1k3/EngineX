//! Sessions avec le faux app-server : scripts tirés des échanges enregistrés
//! (`tests/contract/appserver-*.jsonl`) avec des étapes de contrôle insérées
//! (`script_with`). `cargo build -p enginex-fake-llama` d'abord.

mod common;

use std::time::Duration;

use axum::http::StatusCode;
use common::*;
use serde_json::{Value, json};

fn text_script() -> Vec<Value> {
    script_with("text", |_| {})
}

/// Le tour du script `name` (de `turn/start` à `turn/completed`), ids
/// décalés pour en faire un second tour.
fn second_turn(name: &str) -> Vec<Value> {
    let steps = script_with(name, |_| {});
    let start = step_pos(&steps, 0, is_expect("turn/start"));
    let end = step_pos(&steps, 0, is_send("turn/completed"));
    steps[start..=end]
        .iter()
        .map(|s| {
            let t = s
                .to_string()
                .replace("$turn:1", "$turn:2")
                .replace("$item:1\"", "$item:11\"")
                .replace("$item:2\"", "$item:12\"")
                .replace("$item:3\"", "$item:13\"")
                .replace("$item:4\"", "$item:14\"");
            serde_json::from_str(&t).unwrap()
        })
        .collect()
}

async fn items(e: &Env, sid: &str) -> Vec<Value> {
    e.get(&format!("/api/sessions/{sid}")).await.body["items"]
        .as_array()
        .unwrap()
        .clone()
}

async fn internal_keys(e: &Env) -> Vec<(Vec<u8>, String)> {
    sqlx::query_as("SELECT hash, name FROM api_keys WHERE internal = 1")
        .fetch_all(e.state.db())
        .await
        .unwrap()
}

#[tokio::test]
async fn turn_stores_items_in_order_with_one_user_message() {
    let (e, fake) = env_sessions(&text_script()).await;
    make_active_preset(&e).await;
    let sid = new_session(&e, "auto").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    let snap = ev.until("snapshot", |_| true).await;
    assert_eq!(snap["sandbox"]["state"], "stopped");
    let r = turn(&e, &sid, "Dis bonjour").await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text);
    let t = ev.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["status"], "completed", "{t}");
    // Aucun userMessage de Codex dans le flux.
    for (name, v) in &ev.seen {
        if name == "codex" {
            assert_ne!(v["params"]["item"]["type"], "userMessage", "{v}");
        }
    }
    assert_eq!(
        ev.seen.iter().filter(|(n, _)| n == "user_message").count(),
        1,
        "{:?}",
        ev.names()
    );
    let it = items(&e, &sid).await;
    let kinds: Vec<&str> = it.iter().map(|i| i["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["userMessage", "agentMessage"], "{it:?}");
    assert!(it[0]["item_id"].as_str().unwrap().starts_with("ex-"));
    assert_eq!(it[0]["turn_id"], "turn-1");
    let seqs: Vec<i64> = it.iter().map(|i| i["seq"].as_i64().unwrap()).collect();
    assert_eq!(seqs, [1, 2]);
    // Titre = début du premier message, envoyé aussi à Codex.
    let s = e.get(&format!("/api/sessions/{sid}")).await.body;
    assert_eq!(s["session"]["title"], "Dis bonjour");
    assert_eq!(fake.received("thread/name/set").len(), 1);
    assert!(fake.unexpected().is_empty(), "{:?}", fake.unexpected());
}

#[tokio::test]
async fn approval_policy_follows_the_mode_on_each_turn() {
    let mut script = text_script();
    script.extend(second_turn("text"));
    let (e, fake) = env_sessions(&script).await;
    make_active_preset(&e).await;
    let sid = new_session(&e, "prudent").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    turn(&e, &sid, "un").await;
    ev.until("turn_stored", |_| true).await;
    let r = e
        .send("PATCH", &format!("/api/sessions/{sid}"), json!({"approval": "auto"}))
        .await;
    assert_eq!(r.status, 200);
    turn(&e, &sid, "deux").await;
    ev.until("turn_stored", |v| v["turn"]["id"] == "turn-2").await;
    let starts = fake.received("turn/start");
    let policies: Vec<&Value> = starts.iter().map(|s| &s["params"]["approvalPolicy"]).collect();
    assert_eq!(policies, [&json!("untrusted"), &json!("never")]);
    assert_eq!(
        fake.received("thread/start")[0]["params"]["approvalPolicy"],
        "untrusted"
    );
    assert!(fake.unexpected().is_empty(), "{:?}", fake.unexpected());
}

#[tokio::test]
async fn opening_a_session_starts_no_sandbox() {
    let (e, fake) = env_sessions(&text_script()).await;
    make_active_preset(&e).await;
    let sid = new_session(&e, "auto").await;
    let r = e.get(&format!("/api/sessions/{sid}")).await;
    assert_eq!(r.status, 200);
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    let snap = ev.until("snapshot", |_| true).await;
    assert_eq!(snap["sandbox"]["state"], "stopped");
    assert!(snap["turn"].is_null());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!fake.state.exists(), "un app-server a été lancé");
    assert!(internal_keys(&e).await.is_empty());
}

#[tokio::test]
async fn snapshot_mid_turn_has_partial_text() {
    let script = script_with("text", |s| {
        let i = step_pos(s, 2, is_send("item/agentMessage/delta"));
        s.insert(i + 1, json!({"sleep_ms": 1500}));
    });
    let (e, _fake) = env_sessions(&script).await;
    make_active_preset(&e).await;
    let sid = new_session(&e, "auto").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    turn(&e, &sid, "Dis bonjour").await;
    ev.until("codex", |v| v["params"]["delta"] == " je ").await;
    let mut late = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    let snap = late.until("snapshot", |_| true).await;
    assert_eq!(snap["sandbox"]["state"], "ready");
    let tr = &snap["turn"];
    assert_eq!(tr["id"], "turn-1");
    assert_eq!(tr["items"][0]["type"], "userMessage");
    let partial = tr["partial"].as_object().unwrap();
    let text = partial.values().next().unwrap()["text"].as_str().unwrap();
    assert_eq!(text, "Bonjour, je ");
    // turn_in_progress pendant le tour.
    let r = turn(&e, &sid, "encore").await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert_eq!(r.body["error"], "turn_in_progress");
    ev.until("turn_stored", |_| true).await;
}

#[tokio::test]
async fn approval_is_relayed_answered_and_resolved() {
    let (e, fake) = env_sessions(&script_with("approval", |_| {})).await;
    make_active_preset(&e).await;
    let sid = new_session(&e, "prudent").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    turn(&e, &sid, "Crée made.txt").await;
    let req = ev.until("request", |_| true).await;
    assert_eq!(req["method"], "item/commandExecution/requestApproval");
    let rid = req["rid"].as_str().unwrap().to_string();
    // Une requête en attente figure dans le snapshot.
    let mut late = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    let snap = late.until("snapshot", |_| true).await;
    assert_eq!(snap["requests"][0]["rid"], rid.as_str());
    let r = e
        .send(
            "POST",
            &format!("/api/sessions/{sid}/requests/nope"),
            json!({"decision": "accept"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    let r = e
        .send(
            "POST",
            &format!("/api/sessions/{sid}/requests/{rid}"),
            json!({"decision": "accept"}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    ev.until("request_resolved", |v| v["rid"] == rid.as_str()).await;
    ev.until("turn_stored", |_| true).await;
    let reply = fake
        .log()
        .into_iter()
        .find_map(|l| l.get("reply").cloned())
        .expect("réponse transmise");
    assert_eq!(reply["result"]["decision"], "accept");
    assert!(fake.unexpected().is_empty(), "{:?}", fake.unexpected());
}

#[tokio::test]
async fn interrupt_ends_the_turn_interrupted() {
    let (e, fake) = env_sessions(&script_with("interrupt", |_| {})).await;
    make_active_preset(&e).await;
    let sid = new_session(&e, "auto").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    turn(&e, &sid, "Parle longtemps").await;
    ev.until("codex", |v| v["method"] == "item/agentMessage/delta").await;
    let r = e
        .send("POST", &format!("/api/sessions/{sid}/interrupt"), json!({}))
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    let t = ev.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["status"], "interrupted");
    assert_eq!(fake.received("turn/interrupt").len(), 1);
    assert!(fake.unexpected().is_empty(), "{:?}", fake.unexpected());
}

#[tokio::test]
async fn lost_bridge_fails_the_turn_and_next_turn_resumes_without_duplicates() {
    let script = script_with("resume", |s| {
        let i = step_pos(s, 0, is_send("turn/completed"));
        s[i] = json!({"close": true});
    });
    let (e, fake) = env_sessions(&script).await;
    make_active_preset(&e).await;
    let sid = new_session(&e, "auto").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    turn(&e, &sid, "Premier").await;
    let t = ev.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["status"], "failed");
    assert_eq!(t["turn"]["error"], "sandbox perdue");
    ev.until("sandbox", |v| v["state"] == "stopped").await;
    assert!(internal_keys(&e).await.is_empty(), "clé de la sandbox perdue gardée");
    turn(&e, &sid, "Deuxième").await;
    let t = ev.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["status"], "completed", "{t}");
    assert_eq!(fake.received("thread/resume").len(), 1);
    assert_eq!(fake.received("thread/turns/list").len(), 1);
    let it = items(&e, &sid).await;
    let mut ids: Vec<&str> = it.iter().map(|i| i["item_id"].as_str().unwrap()).collect();
    let n = ids.len();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), n, "doublons : {it:?}");
    assert_eq!(it.iter().filter(|i| i["kind"] == "agentMessage").count(), 2, "{it:?}");
    assert_eq!(it.iter().filter(|i| i["kind"] == "userMessage").count(), 2);
    let turns = e.get(&format!("/api/sessions/{sid}")).await.body["turns"].clone();
    assert_eq!(turns[0]["status"], "failed");
    assert!(fake.unexpected().is_empty(), "{:?}", fake.unexpected());
}

#[tokio::test]
async fn refusals_before_a_turn() {
    let (e, _fake) = env_sessions(&text_script()).await;
    let sid = new_session(&e, "auto").await;
    let r = turn(&e, &sid, "x").await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert_eq!(r.body["error"], "no_active_preset");
    make_active_preset(&e).await;
    let up = upload(&e, &format!("/api/sessions/{sid}/attachments"), "image/png", png(8, 8)).await;
    assert_eq!(up.status, 200, "{}", up.text);
    let r = e
        .send(
            "POST",
            &format!("/api/sessions/{sid}/turns"),
            json!({"text": "x", "attachments": [up.body["id"]]}),
        )
        .await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert_eq!(r.body["error"], "no_vision");
}

#[tokio::test]
async fn engine_not_ready_is_refused() {
    let (e, _fake) = env_sessions_cfg(&text_script(), "FAKE_LOAD_MS=8000", |_| {}).await;
    let r = e
        .send("POST", "/api/downloads", json!({"repo": REPO, "quant": "Q8_0"}))
        .await;
    assert_eq!(r.status, 200);
    e.until("/api/downloads", |v| v[0]["state"] == "done").await;
    let id = e.until("/api/presets", |v| v[0]["id"].is_i64()).await[0]["id"]
        .as_i64()
        .unwrap();
    e.send("POST", &format!("/api/presets/{id}/activate"), json!({})).await;
    let sid = new_session(&e, "auto").await;
    let r = turn(&e, &sid, "x").await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE, "{}", r.text);
    assert_eq!(r.body["error"], "engine_not_ready");
}

#[tokio::test]
async fn restart_revokes_the_old_key_and_a_new_preset_restarts_the_sandbox() {
    let (e, fake) = env_sessions(&script_with("resume", |_| {})).await;
    let pid = make_active_preset(&e).await;
    let sid = new_session(&e, "auto").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    turn(&e, &sid, "Premier").await;
    ev.until("turn_stored", |_| true).await;
    let k1 = internal_keys(&e).await;
    assert_eq!(k1.len(), 1);
    assert_eq!(k1[0].1, format!("sandbox:{sid}"));
    // Même preset, autre contexte : la sandbox redémarre au tour suivant.
    let p = e.get("/api/presets").await.body[0].clone();
    let mut settings = p["settings"].clone();
    settings["ctx"] = json!(settings["ctx"].as_u64().unwrap() / 2);
    let r = e
        .send(
            "PUT",
            &format!("/api/presets/{pid}"),
            json!({"name": p["name"], "settings": settings}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    e.until("/api/engine", |v| v["state"]["state"] == "ready").await;
    turn(&e, &sid, "Deuxième").await;
    ev.until("sandbox", |v| v["state"] == "restarting").await;
    let t = ev.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["status"], "completed", "{t}");
    let k2 = internal_keys(&e).await;
    assert_eq!(k2.len(), 1, "ancienne clé non révoquée : {k2:?}");
    assert_ne!(k1[0].0, k2[0].0);
    assert_eq!(std::fs::read_to_string(&fake.state).unwrap().trim(), "2");
    // Le config.toml suit le nouveau contexte.
    let toml = std::fs::read_to_string(e.dir.path().join(format!("data/sessions/{sid}/codex/config.toml"))).unwrap();
    assert!(
        toml.contains(&format!("model_context_window = {}", settings["ctx"])),
        "{toml}"
    );
}

#[tokio::test]
async fn idle_sandbox_is_stopped() {
    let (e, _fake) = env_sessions_cfg(&text_script(), "", |c| {
        c.timings.sandbox.idle_override = Some(Duration::from_millis(500));
    })
    .await;
    make_active_preset(&e).await;
    let sid = new_session(&e, "auto").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    turn(&e, &sid, "x").await;
    ev.until("turn_stored", |_| true).await;
    let s = ev.until("sandbox", |v| v["state"] == "stopped").await;
    assert_eq!(s["detail"], "inactivité");
    assert!(internal_keys(&e).await.is_empty());
}

#[tokio::test]
async fn startup_cleanup_only_touches_this_instance() {
    let (e, _fake) = env_sessions(&text_script()).await;
    let instance: String = sqlx::query_scalar("SELECT value FROM settings WHERE key = 'instance.id'")
        .fetch_one(e.state.db())
        .await
        .unwrap();
    let dir = e.dir.path().join("data/sandboxes-local");
    std::fs::create_dir_all(&dir).unwrap();
    use std::os::unix::process::CommandExt;
    let mut ours = std::process::Command::new("sleep")
        .arg("60")
        .process_group(0)
        .spawn()
        .unwrap();
    let mut theirs = std::process::Command::new("sleep").arg("60").spawn().unwrap();
    std::fs::write(
        dir.join("ours.json"),
        json!({"pid": ours.id(), "owner": instance, "preset": "1:0"}).to_string(),
    )
    .unwrap();
    std::fs::write(
        dir.join("theirs.json"),
        json!({"pid": theirs.id(), "owner": "autre-instance", "preset": "1:0"}).to_string(),
    )
    .unwrap();
    let sid = new_session(&e, "auto").await;
    sqlx::query("INSERT INTO api_keys (name, hash, created_at, internal) VALUES ('sandbox:x', x'00', 0, 1)")
        .execute(e.state.db())
        .await
        .unwrap();
    sqlx::query("INSERT INTO chat_turns (id, session_id, status, started_at) VALUES ('t1', ?, 'inProgress', 0)")
        .bind(&sid)
        .execute(e.state.db())
        .await
        .unwrap();
    enginex_server::sandboxes::boot(&e.state).await.unwrap();
    std::thread::sleep(Duration::from_millis(200));
    assert!(ours.try_wait().unwrap().is_some(), "sandbox de l'instance gardée");
    assert!(
        theirs.try_wait().unwrap().is_none(),
        "sandbox d'une autre instance tuée"
    );
    let _ = theirs.kill();
    assert!(dir.join("theirs.json").exists());
    assert!(internal_keys(&e).await.is_empty());
    let turns = e.get(&format!("/api/sessions/{sid}")).await.body["turns"].clone();
    assert_eq!(turns[0]["status"], "interrupted");
}

#[tokio::test]
async fn replaced_secret_stays_masked_while_its_sandbox_lives() {
    let (e, _fake) = env_sessions(&text_script()).await;
    make_active_preset(&e).await;
    let old = "ancien-secret-42";
    let r = e
        .send(
            "PUT",
            "/api/vault",
            json!({"scope": "global", "name": "MY_TOKEN", "value": old}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    let sid = new_session(&e, "auto").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    turn(&e, &sid, "x").await;
    ev.until("turn_stored", |_| true).await;
    e.send(
        "PUT",
        "/api/vault",
        json!({"scope": "global", "name": "MY_TOKEN", "value": "nouveau-secret-43"}),
    )
    .await;
    assert_eq!(e.state.masker().mask(&format!("a {old} b")), "a *** b");
    assert_eq!(e.state.masker().mask("a nouveau-secret-43 b"), "a *** b");
    e.send("PATCH", &format!("/api/sessions/{sid}"), json!({"archived": true}))
        .await;
    ev.until("sandbox", |v| v["state"] == "stopped").await;
    assert_eq!(e.state.masker().mask(&format!("a {old} b")), format!("a {old} b"));
}

#[tokio::test]
async fn secrets_are_masked_in_the_stream_and_in_storage() {
    // Secret coupé sur trois deltas : « Bonj|our,| je |suis… ».
    let secret = "jour, je s";
    let (e, _fake) = env_sessions(&text_script()).await;
    make_active_preset(&e).await;
    let r = e
        .send(
            "PUT",
            "/api/vault",
            json!({"scope": "global", "name": "SPLIT", "value": secret}),
        )
        .await;
    assert_eq!(r.status, 200);
    let sid = new_session(&e, "auto").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    turn(&e, &sid, &format!("voici {secret}")).await;
    ev.until("turn_stored", |_| true).await;
    let streamed: String = ev
        .seen
        .iter()
        .filter(|(n, v)| n == "codex" && v["method"] == "item/agentMessage/delta")
        .map(|(_, v)| v["params"]["delta"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(streamed, "Bon***uis prêt.");
    for (_, v) in &ev.seen {
        assert!(!v.to_string().contains(secret), "{v}");
    }
    let stored = e.get(&format!("/api/sessions/{sid}")).await.text;
    assert!(!stored.contains(secret), "{stored}");
    assert!(stored.contains("Bon***uis prêt."));
    assert!(stored.contains("voici ***"));
}

#[tokio::test]
async fn delete_removes_sandbox_key_rows_and_files_but_not_the_topic() {
    let (e, _fake) = env_sessions(&text_script()).await;
    make_active_preset(&e).await;
    let sid = new_session(&e, "auto").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    turn(&e, &sid, "x").await;
    ev.until("turn_stored", |_| true).await;
    let topic = e.dir.path().join("data/workspace/projects/general/topics/inbox");
    std::fs::write(topic.join("code.rs"), "fn main() {}").unwrap();
    let sdir = e.dir.path().join(format!("data/sessions/{sid}"));
    assert!(sdir.exists());
    assert_eq!(internal_keys(&e).await.len(), 1);
    let r = e.send("DELETE", &format!("/api/sessions/{sid}"), json!({})).await;
    assert_eq!(r.status, 200, "{}", r.text);
    assert!(!sdir.exists());
    assert!(internal_keys(&e).await.is_empty());
    assert!(topic.join("code.rs").exists());
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chat_items WHERE session_id = ?")
        .bind(&sid)
        .fetch_one(e.state.db())
        .await
        .unwrap();
    assert_eq!(n, 0);
    assert_eq!(
        e.get(&format!("/api/sessions/{sid}")).await.status,
        StatusCode::NOT_FOUND
    );
    assert!(
        std::fs::read_dir(e.dir.path().join("data/sandboxes-local"))
            .unwrap()
            .all(|f| { !f.unwrap().file_name().to_string_lossy().ends_with(".json") })
    );
}

#[tokio::test]
async fn file_route_refuses_escapes_symlinks_and_svg() {
    let (e, _fake) = env_sessions(&text_script()).await;
    let sid = new_session(&e, "auto").await;
    let topic = e.dir.path().join("data/workspace/projects/general/topics/inbox");
    std::fs::write(topic.join("ok.png"), png(4, 4)).unwrap();
    std::fs::write(topic.join("x.svg"), "<svg onload=alert(1)/>").unwrap();
    std::fs::write(e.dir.path().join("outside.png"), png(4, 4)).unwrap();
    std::os::unix::fs::symlink(e.dir.path().join("outside.png"), topic.join("link.png")).unwrap();
    for (p, want) in [
        ("ok.png", StatusCode::OK),
        ("/workspace/ok.png", StatusCode::OK),
        ("../../../../outside.png", StatusCode::NOT_FOUND),
        ("link.png", StatusCode::NOT_FOUND),
        ("x.svg", StatusCode::OK),
        ("/etc/passwd", StatusCode::NOT_FOUND),
    ] {
        let r = e.get(&format!("/api/sessions/{sid}/file?path={p}")).await;
        assert_eq!(r.status, want, "{p}");
    }
    // Le svg est servi isolé : CSP `sandbox`, aucun script.
    let r = e.get(&format!("/api/sessions/{sid}/file?path=x.svg")).await;
    let csp = r.headers["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("sandbox"), "{csp}");
    assert_eq!(r.headers["x-content-type-options"], "nosniff");
}

#[tokio::test]
async fn attachment_limits() {
    let (e, _fake) = env_sessions(&text_script()).await;
    let sid = new_session(&e, "auto").await;
    let path = format!("/api/sessions/{sid}/attachments");
    let big = vec![0u8; 20 * 1024 * 1024 + 512 * 1024];
    let r = upload(&e, &path, "image/png", big).await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(r.body["error"], "attachment_invalid");
    let r = upload(&e, &path, "text/plain", b"hello".to_vec()).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.body["error"], "attachment_invalid");
    // Grande image réduite à 1568 px, JPEG sans transparence.
    let r = upload(&e, &path, "image/png", png(3000, 1000)).await;
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(r.body["width"], 1568);
    assert!(r.body["id"].as_str().unwrap().ends_with(".jpg"));
    let got = e.get(r.body["url"].as_str().unwrap()).await;
    assert_eq!(got.status, 200);
}

// ---- Étape 4b : instantanés, retours arrière, vérification ----

fn topic_dir(e: &Env) -> std::path::PathBuf {
    e.dir.path().join("data/workspace/projects/general/topics/inbox")
}

/// Le script `name` avec une pause avant `turn/completed` (le test écrit des
/// fichiers pendant le tour).
fn slow(steps: Vec<Value>) -> Vec<Value> {
    let mut steps = steps;
    let i = step_pos(&steps, 0, is_send("turn/completed"));
    steps.insert(i, json!({"sleep_ms": 700}));
    steps
}

async fn session_body(e: &Env, sid: &str) -> Value {
    e.get(&format!("/api/sessions/{sid}")).await.body
}

/// Lance un tour et écrit `files` pendant qu'il tourne ; rend l'id du tour
/// une fois ses modifications stockées.
async fn turn_writing(e: &Env, ev: &mut Sse, sid: &str, text: &str, files: &[(&str, &str)]) -> String {
    let r = turn(e, sid, text).await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text);
    ev.until("codex", |v| v["method"] == "turn/started").await;
    for (p, c) in files {
        std::fs::write(topic_dir(e).join(p), c).unwrap();
    }
    let ch = ev.until("changes", |_| true).await;
    ch["turn_id"].as_str().unwrap().to_string()
}

async fn wait_for(f: impl Fn() -> bool) {
    for _ in 0..100 {
        if f() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("condition jamais vraie");
}

#[tokio::test]
async fn files_only_revert_adds_a_note_and_redo_works() {
    let mut script = slow(text_script());
    script.extend(slow(second_turn("text")));
    let (e, fake) = env_sessions(&script).await;
    make_active_preset(&e).await;
    let sid = new_session(&e, "auto").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    let tid = turn_writing(&e, &mut ev, &sid, "Écris a.txt", &[("a.txt", "v1")]).await;
    let s = session_body(&e, &sid).await;
    let n_items = s["items"].as_array().unwrap().len();
    let t = &s["turns"][0];
    assert_eq!(t["has_snapshot"], true);
    assert_eq!(t["changes"]["files"][0]["path"], "a.txt");
    assert_eq!(t["changes"]["files"][0]["status"], "A");
    let d = e.get(&format!("/api/sessions/{sid}/turns/{tid}/diff?path=a.txt")).await;
    assert_eq!(d.status, 200, "{}", d.text);
    assert!(d.body["diff"].as_str().unwrap().contains("+v1"), "{}", d.text);
    let r = e
        .send(
            "POST",
            &format!("/api/sessions/{sid}/turns/{tid}/revert"),
            json!({"conversation": false}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    assert!(!topic_dir(&e).join("a.txt").exists());
    let s = session_body(&e, &sid).await;
    assert_eq!(s["session"]["undo"], true);
    assert!(s["session"]["pending_note"].as_str().unwrap().contains("Écris a.txt"));
    assert_eq!(s["items"].as_array().unwrap().len(), n_items, "la conversation reste");
    // « Rétablir ».
    let r = e
        .send("POST", &format!("/api/sessions/{sid}/restore-undo"), json!({}))
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(std::fs::read_to_string(topic_dir(&e).join("a.txt")).unwrap(), "v1");
    // La note part en tête du tour suivant, puis est vidée ; le bandeau aussi.
    turn(&e, &sid, "Suite").await;
    ev.until("turn_stored", |_| true).await;
    let starts = fake.received("turn/start");
    let first = starts[1]["params"]["input"][0]["text"].as_str().unwrap();
    assert!(first.starts_with("Note d'EngineX"), "{first}");
    let s = session_body(&e, &sid).await;
    assert!(s["session"]["pending_note"].is_null());
    assert_eq!(s["session"]["undo"], false);
    assert!(fake.unexpected().is_empty(), "{:?}", fake.unexpected());
}

#[tokio::test]
async fn conversation_revert_with_a_live_sandbox() {
    let mut script = slow(text_script());
    script.push(json!({"expect": "thread/revert", "result": {}}));
    // Conversation vidée : le message suivant redonne son titre à la session.
    let mut next = second_turn("text");
    let i = step_pos(&next, 0, is_expect("turn/start"));
    next.insert(i, json!({"expect": "thread/name/set", "result": {}}));
    script.extend(next);
    let (e, fake) = env_sessions(&script).await;
    make_active_preset(&e).await;
    let sid = new_session(&e, "auto").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    let tid = turn_writing(&e, &mut ev, &sid, "Premier", &[("b.txt", "x")]).await;
    let r = e
        .send(
            "POST",
            &format!("/api/sessions/{sid}/turns/{tid}/revert"),
            json!({"conversation": true}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(r.body["user"]["text"], "Premier");
    assert!(!topic_dir(&e).join("b.txt").exists());
    let s = session_body(&e, &sid).await;
    assert!(s["items"].as_array().unwrap().is_empty(), "{}", s["items"]);
    assert!(s["turns"].as_array().unwrap().is_empty());
    wait_for(|| fake.received("thread/revert").len() == 1).await;
    assert_eq!(
        fake.received("thread/revert")[0]["params"]["beforeTurnId"],
        tid.as_str()
    );
    e.until(&format!("/api/sessions/{sid}"), |v| {
        v["session"]["pending_revert"] == false
    })
    .await;
    // L'UI l'apprend aussi : le bandeau « en attente » disparaît.
    let mut applied = false;
    for _ in 0..50 {
        if ev.last("session").is_some_and(|s| s["pending_revert"] == false) {
            applied = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(applied, "{:?}", ev.last("session"));
    turn(&e, &sid, "Deuxième").await;
    let t = ev.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["status"], "completed", "{t}");
    assert!(fake.unexpected().is_empty(), "{:?}", fake.unexpected());
}

#[tokio::test]
async fn refused_revert_is_shown_and_the_turn_goes_on() {
    let mut script = slow(text_script());
    script.push(json!({"expect": "thread/revert", "error": {"code": -32602, "message": "tour inconnu"}}));
    script.extend(second_turn("text"));
    let (e, fake) = env_sessions(&script).await;
    make_active_preset(&e).await;
    let sid = new_session(&e, "auto").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    let tid = turn_writing(&e, &mut ev, &sid, "Premier", &[("c.txt", "x")]).await;
    e.send(
        "POST",
        &format!("/api/sessions/{sid}/turns/{tid}/revert"),
        json!({"conversation": true}),
    )
    .await;
    let n = ev.until("notice", |_| true).await;
    assert!(n["item"]["text"].as_str().unwrap().contains("tour inconnu"), "{n}");
    let s = session_body(&e, &sid).await;
    assert_eq!(s["session"]["pending_revert"], false);
    assert_eq!(s["items"][0]["kind"], "notice");
    turn(&e, &sid, "Deuxième").await;
    let t = ev.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["status"], "completed", "{t}");
    assert!(fake.unexpected().is_empty(), "{:?}", fake.unexpected());
}

/// Script `resume` : segment 1 (premier lancement), segment 2 (relance).
fn resume_segments() -> (Vec<Value>, Vec<Value>) {
    let steps = script_with("resume", |_| {});
    let i = steps.iter().position(|s| s.get("launch").is_some()).unwrap();
    (steps[..i].to_vec(), steps[i + 1..].to_vec())
}

fn with_revert_after_resume(mut seg: Vec<Value>) -> Vec<Value> {
    let i = step_pos(&seg, 0, is_expect("thread/resume"));
    seg.insert(i + 1, json!({"expect": "thread/revert", "result": {}}));
    seg
}

#[tokio::test]
async fn conversation_revert_while_stopped_is_sent_before_catch_up() {
    let (seg1, seg2) = resume_segments();
    let mut script = slow(seg1);
    script.push(json!({"launch": true}));
    script.extend(with_revert_after_resume(seg2));
    let (e, fake) = env_sessions(&script).await;
    make_active_preset(&e).await;
    let sid = new_session(&e, "auto").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    let tid = turn_writing(&e, &mut ev, &sid, "Premier", &[("d.txt", "x")]).await;
    enginex_server::bridge::hub(&e.state, &sid)
        .stop_conn(&e.state, "test")
        .await;
    let r = e
        .send(
            "POST",
            &format!("/api/sessions/{sid}/turns/{tid}/revert"),
            json!({"conversation": true}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(
        std::fs::read_to_string(&fake.state).unwrap().trim(),
        "1",
        "aucune sandbox démarrée"
    );
    assert_eq!(session_body(&e, &sid).await["session"]["pending_revert"], true);
    turn(&e, &sid, "Deuxième").await;
    let t = ev.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["status"], "completed", "{t}");
    let methods: Vec<String> = fake
        .log()
        .iter()
        .filter_map(|l| l["received"]["method"].as_str().map(str::to_string))
        .collect();
    let rv = methods.iter().position(|m| m == "thread/revert").unwrap();
    let tl = methods.iter().position(|m| m == "thread/turns/list").unwrap();
    assert!(rv < tl, "{methods:?}");
    // `thread/turns/list` rend encore le tour retiré : il ne revient pas.
    let s = session_body(&e, &sid).await;
    assert!(
        s["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["turn_id"] != tid.as_str()),
        "{}",
        s["items"]
    );
    assert!(s["turns"].as_array().unwrap().iter().all(|t| t["id"] != tid.as_str()));
    assert_eq!(s["session"]["pending_revert"], false);
}

#[tokio::test]
async fn revert_lost_with_the_connection_is_kept_and_retried() {
    let (seg1, seg2) = resume_segments();
    let mut script = slow(seg1);
    script.push(json!({"launch": true}));
    script.extend(seg2[..=step_pos(&seg2, 0, is_expect("thread/resume"))].to_vec());
    script.push(json!({"close": true}));
    script.push(json!({"launch": true}));
    script.extend(with_revert_after_resume(seg2));
    let (e, fake) = env_sessions(&script).await;
    make_active_preset(&e).await;
    let sid = new_session(&e, "auto").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    let tid = turn_writing(&e, &mut ev, &sid, "Premier", &[("e.txt", "x")]).await;
    enginex_server::bridge::hub(&e.state, &sid)
        .stop_conn(&e.state, "test")
        .await;
    e.send(
        "POST",
        &format!("/api/sessions/{sid}/turns/{tid}/revert"),
        json!({"conversation": true}),
    )
    .await;
    turn(&e, &sid, "Deuxième").await;
    let t = ev.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["status"], "failed");
    assert!(
        t["turn"]["error"]
            .as_str()
            .unwrap()
            .contains("retour de conversation non appliqué"),
        "{t}"
    );
    assert_eq!(session_body(&e, &sid).await["session"]["pending_revert"], true);
    turn(&e, &sid, "Troisième").await;
    let t = ev.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["status"], "completed", "{t}");
    assert_eq!(session_body(&e, &sid).await["session"]["pending_revert"], false);
    // Le faux app-server peut avoir lu le premier envoi juste avant de couper.
    let n = fake.received("thread/revert").len();
    assert!((1..=2).contains(&n), "{n}");
}

#[tokio::test]
async fn revert_timed_out_while_connected_is_resent_before_the_next_turn() {
    let mut script = slow(text_script());
    script.push(json!({"expect": "thread/revert", "defer": true}));
    script.push(json!({"expect": "thread/revert", "result": {}}));
    script.extend(second_turn("text"));
    let (e, fake) = env_sessions(&script).await;
    make_active_preset(&e).await;
    let sid = new_session(&e, "auto").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    let tid = turn_writing(&e, &mut ev, &sid, "Premier", &[("f.txt", "x")]).await;
    e.send(
        "POST",
        &format!("/api/sessions/{sid}/turns/{tid}/revert"),
        json!({"conversation": true}),
    )
    .await;
    wait_for(|| fake.received("thread/revert").len() == 1).await;
    // Délai du client (5 s en test) : le retour reste en attente.
    tokio::time::sleep(std::time::Duration::from_millis(5500)).await;
    assert_eq!(session_body(&e, &sid).await["session"]["pending_revert"], true);
    turn(&e, &sid, "Deuxième").await;
    let t = ev.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["status"], "completed", "{t}");
    assert_eq!(fake.received("thread/revert").len(), 2);
    assert_eq!(session_body(&e, &sid).await["session"]["pending_revert"], false);
}

#[tokio::test]
async fn folder_busy_and_no_snapshot() {
    let mut script = slow(text_script());
    script.push(json!({"launch": true}));
    script.extend(slow(text_script()));
    let (e, _fake) = env_sessions(&script).await;
    make_active_preset(&e).await;
    let a = new_session(&e, "auto").await;
    let b = new_session(&e, "auto").await;
    let mut ev_a = sse(&e, &format!("/api/sessions/{a}/events")).await;
    let mut ev_b = sse(&e, &format!("/api/sessions/{b}/events")).await;
    let tid = turn_writing(&e, &mut ev_a, &a, "Premier", &[("g.txt", "x")]).await;
    // Un tour d'une autre session du même topic : refus.
    turn(&e, &b, "Autre").await;
    ev_b.until("codex", |v| v["method"] == "turn/started").await;
    let r = e
        .send(
            "POST",
            &format!("/api/sessions/{a}/turns/{tid}/revert"),
            json!({"conversation": false}),
        )
        .await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert_eq!(r.body["error"], "folder_busy");
    let r = e
        .send(
            "POST",
            &format!("/api/sessions/{a}/turns/{tid}/undo-file"),
            json!({"path": "g.txt"}),
        )
        .await;
    assert_eq!(r.body["error"], "folder_busy");
    ev_b.until("turn_stored", |_| true).await;
    ev_b.until("changes", |_| true).await;
    // Plus d'état vert, plus d'instantané.
    let r = e
        .send("POST", &format!("/api/sessions/{a}/restore-green"), json!({}))
        .await;
    assert_eq!(r.body["error"], "no_snapshot");
    let r = e
        .send("POST", &format!("/api/sessions/{a}/restore-undo"), json!({}))
        .await;
    assert_eq!(r.body["error"], "no_snapshot");
    sqlx::query("UPDATE chat_turns SET snap_pre = NULL WHERE id = ?")
        .bind(&tid)
        .execute(e.state.db())
        .await
        .unwrap();
    let r = e
        .send(
            "POST",
            &format!("/api/sessions/{a}/turns/{tid}/revert"),
            json!({"conversation": false}),
        )
        .await;
    assert_eq!(r.body["error"], "no_snapshot");
    let r = e
        .send("POST", &format!("/api/sessions/{a}/verify/run"), json!({}))
        .await;
    assert_eq!(r.body["error"], "no_recipe");
}

#[tokio::test]
async fn a_turn_waits_for_a_restore_on_the_same_folder() {
    let (e, fake) = env_sessions(&text_script()).await;
    make_active_preset(&e).await;
    let sid = new_session(&e, "auto").await;
    let row = enginex_server::sessions::row(&e.state, &sid).await.unwrap();
    let f = enginex_server::snapshots::folder(&e.state, &row).await.unwrap();
    let guard = enginex_server::snapshots::lock(&e.state, &f).await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    turn(&e, &sid, "Premier").await;
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    assert!(
        fake.received("turn/start").is_empty(),
        "le tour est parti pendant le retour arrière"
    );
    drop(guard);
    let t = ev.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["status"], "completed", "{t}");
}

fn exec_result(code: i64, out: &str) -> Value {
    json!({"exitCode": code, "stdout": out, "stderr": ""})
}

#[tokio::test]
async fn failed_verification_starts_a_fix_turn_then_goes_green() {
    let mut script = slow(text_script());
    script.push(json!({"expect": "command/exec", "result": exec_result(1, "ok.txt absent")}));
    script.extend(second_turn("text"));
    script.push(json!({"expect": "command/exec", "result": exec_result(0, "")}));
    let (e, fake) = env_sessions(&script).await;
    make_active_preset(&e).await;
    std::fs::create_dir_all(topic_dir(&e).join(".enginex")).unwrap();
    std::fs::write(
        topic_dir(&e).join(".enginex/verify.toml"),
        "[[step]]\nname = \"fichier\"\nrun = \"test -f ok.txt\"\n",
    )
    .unwrap();
    let sid = new_session(&e, "auto").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    let t1 = turn_writing(&e, &mut ev, &sid, "Fais-le", &[("h.txt", "x")]).await;
    let v = ev.until("verify", |v| v["status"] == "failed").await;
    assert_eq!(v["attempt"], 1);
    assert_eq!(v["steps"][0]["exit_code"], 1);
    let v = ev.until("verify", |v| v["status"] == "passed").await;
    assert_eq!(v["attempt"], 2);
    let exec = &fake.received("command/exec")[0]["params"];
    assert_eq!(exec["command"], json!(["bash", "-lc", "test -f ok.txt"]));
    assert_eq!(exec["timeoutMs"], 600_000);
    let starts = fake.received("turn/start");
    let fix = starts[1]["params"]["input"][0]["text"].as_str().unwrap();
    assert!(
        fix.starts_with("La vérification automatique d'EngineX a échoué (essai 1, correction 1/3)"),
        "{fix}"
    );
    assert!(fix.contains("ok.txt absent"));
    let s = session_body(&e, &sid).await;
    let turns = s["turns"].as_array().unwrap();
    assert_eq!(turns[0]["id"], t1.as_str());
    assert_eq!(turns[0]["verifications"][0]["status"], "failed");
    assert_eq!(turns[1]["origin"], "verify");
    assert_eq!(turns[1]["verifications"][0]["status"], "passed");
    assert_eq!(s["green_turn"], turns[1]["id"]);
    assert!(fake.unexpected().is_empty(), "{:?}", fake.unexpected());
}

#[tokio::test]
async fn a_message_cancels_a_running_verification() {
    let mut script = slow(text_script());
    script.push(json!({"expect": "command/exec", "defer": true}));
    script.push(json!({"expect": "command/exec/terminate"}));
    script.push(json!({"answer": "command/exec", "result": exec_result(-1, "")}));
    script.extend(second_turn("text"));
    let (e, fake) = env_sessions(&script).await;
    make_active_preset(&e).await;
    std::fs::create_dir_all(topic_dir(&e).join(".enginex")).unwrap();
    std::fs::write(
        topic_dir(&e).join(".enginex/verify.toml"),
        "[[step]]\nname = \"long\"\nrun = \"sleep 100\"\n",
    )
    .unwrap();
    let sid = new_session(&e, "auto").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    turn_writing(&e, &mut ev, &sid, "Premier", &[("i.txt", "x")]).await;
    ev.until("verify", |v| v["status"] == "running").await;
    wait_for(|| fake.received("command/exec").len() == 1).await;
    let r = turn(&e, &sid, "Deuxième").await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text);
    let v = ev.until("verify", |v| v["status"] == "cancelled").await;
    assert_eq!(v["attempt"], 1);
    let t = ev.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["status"], "completed", "{t}");
    assert_eq!(t["turn"]["origin"], "user");
    assert!(fake.unexpected().is_empty(), "{:?}", fake.unexpected());
}

#[tokio::test]
async fn interrupt_cancels_a_verification_and_boot_marks_running_ones_as_errors() {
    let mut script = slow(text_script());
    script.push(json!({"expect": "command/exec", "defer": true}));
    script.push(json!({"expect": "command/exec/terminate"}));
    script.push(json!({"answer": "command/exec", "result": exec_result(-1, "")}));
    let (e, fake) = env_sessions(&script).await;
    make_active_preset(&e).await;
    std::fs::create_dir_all(topic_dir(&e).join(".enginex")).unwrap();
    std::fs::write(
        topic_dir(&e).join(".enginex/verify.toml"),
        "[[step]]\nname = \"long\"\nrun = \"sleep 100\"\n",
    )
    .unwrap();
    let sid = new_session(&e, "auto").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    turn_writing(&e, &mut ev, &sid, "Premier", &[("j.txt", "x")]).await;
    wait_for(|| fake.received("command/exec").len() == 1).await;
    let r = e
        .send("POST", &format!("/api/sessions/{sid}/interrupt"), json!({}))
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    ev.until("verify", |v| v["status"] == "cancelled").await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(fake.received("turn/start").len(), 1, "aucun tour de correction");
    // Une vérification restée `running` passe en `error` au démarrage.
    sqlx::query(
        "INSERT INTO verifications (session_id, turn_id, attempt, status, steps, started_at) \
         VALUES (?, 'x', 1, 'running', '[]', 0)",
    )
    .bind(&sid)
    .execute(e.state.db())
    .await
    .unwrap();
    enginex_server::sandboxes::boot(&e.state).await.unwrap();
    let st: String = sqlx::query_scalar("SELECT status FROM verifications WHERE turn_id = 'x'")
        .fetch_one(e.state.db())
        .await
        .unwrap();
    assert_eq!(st, "error");
}

#[tokio::test]
async fn terminal_requires_the_same_origin() {
    let (e, _fake) = env_sessions(&text_script()).await;
    let sid = new_session(&e, "auto").await;
    let base = serve_tcp(e.router.clone()).await;
    let path = format!("/api/sessions/{sid}/terminal");
    assert_eq!(ws_connect(&base, &e.cookie, &path, None).await.err(), Some(403));
    assert_eq!(
        ws_connect(&base, &e.cookie, &path, Some("http://evil.example"))
            .await
            .err(),
        Some(403)
    );
    assert_eq!(
        ws_connect(&base, "enginex_session=x", &path, Some(&base)).await.err(),
        Some(401)
    );
}
