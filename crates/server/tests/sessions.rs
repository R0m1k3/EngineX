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
        ("x.svg", StatusCode::NOT_FOUND),
        ("/etc/passwd", StatusCode::NOT_FOUND),
    ] {
        let r = e.get(&format!("/api/sessions/{sid}/file?path={p}")).await;
        assert_eq!(r.status, want, "{p}");
    }
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
