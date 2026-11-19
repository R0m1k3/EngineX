//! Mémoire (API, versions, suppression, migration), recall (FTS, filtres),
//! pont (instructions, renvoi du bloc) et tour de fin de session.

mod common;

use std::time::Duration;

use axum::{http::StatusCode, response::IntoResponse};
use common::{
    Env, FakeAs, env, env_sessions_cfg, inbox, is_expect, is_send, make_active_preset, new_session, script_with, sse,
    step_pos, turn,
};
use enginex_server::{error::ApiError, recall, state::now};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

async fn put(e: &Env, path: &str, file: &str, content: &str) -> common::Resp {
    let cur = e.get(&format!("/api/memory/file?path={path}&file={file}")).await;
    let sha = cur.body["sha"].as_str().unwrap().to_string();
    e.send(
        "PUT",
        &format!("/api/memory/file?path={path}&file={file}"),
        json!({"content": content, "base_sha": sha}),
    )
    .await
}

#[tokio::test]
async fn file_api_versions_and_limits() {
    let e = env().await;
    let r = e.get("/api/memory/file?path=general/inbox&file=memory").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.body["content"], "");
    let empty_sha = r.body["sha"].as_str().unwrap().to_string();

    let r = put(&e, "general/inbox", "memory", "- un fait\n").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let f = e.dir.path().join("data/workspace/memory/general/inbox/memory.md");
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "- un fait\n");
    let r = e.get("/api/memory/file?path=general/inbox&file=memory").await;
    assert_eq!(r.body["content"], "- un fait\n");

    // base_sha périmé.
    let r = e
        .send(
            "PUT",
            "/api/memory/file?path=general/inbox&file=memory",
            json!({"content": "x", "base_sha": empty_sha}),
        )
        .await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert_eq!(r.body["error"], "memory_changed");

    // Limites : 16 Kio pour memory.md, 4 Kio pour instructions.md.
    let r = put(&e, "general/inbox", "memory", &"x".repeat(16385)).await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(r.body["error"], "too_large");
    let r = put(&e, "", "instructions", &"x".repeat(4097)).await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);
    let r = put(&e, "", "instructions", "Réponds en français.").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert!(e.dir.path().join("data/workspace/memory/instructions.md").is_file());

    // Niveau inconnu, fichier inconnu.
    let r = e.get("/api/memory/file?path=nope&file=memory").await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    let r = e.get("/api/memory/file?path=general/nope&file=memory").await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    let r = e.get("/api/memory/file?path=general/inbox/x&file=memory").await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    let r = e.get("/api/memory/file?path=general&file=agents").await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);

    // Versions : 21 écritures → 20 gardées, la plus récente en tête.
    for i in 0..20 {
        let r = put(&e, "general", "memory", &format!("- version {i}\n")).await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    }
    let r = put(&e, "general", "memory", "- version finale\n").await;
    assert_eq!(r.status, StatusCode::OK);
    let v = e.get("/api/memory/revisions?path=general&file=memory").await.body;
    let list = v["revisions"].as_array().unwrap();
    assert_eq!(list.len(), 20);
    assert_eq!(list[0]["author"], "michael");
    let old_id = list[5]["id"].as_i64().unwrap();
    let old = e.get(&format!("/api/memory/revisions/{old_id}")).await.body;
    assert_eq!(old["content"], "- version 15\n");
    // Restauration = PUT de l'ancien contenu.
    let r = put(&e, "general", "memory", old["content"].as_str().unwrap()).await;
    assert_eq!(r.status, StatusCode::OK);
    let r = e.get("/api/memory/file?path=general&file=memory").await;
    assert_eq!(r.body["content"], "- version 15\n");
    assert_eq!(
        e.get("/api/memory/revisions/999999").await.status,
        StatusCode::NOT_FOUND
    );

    // Arbre : tailles, limites, auteur.
    let t = e.get("/api/memory").await.body;
    assert_eq!(t["global"]["instructions"]["limit"], 4096);
    assert_eq!(t["projects"][0]["slug"], "general");
    assert_eq!(t["projects"][0]["memory"]["size"], 13);
    assert_eq!(t["projects"][0]["memory"]["author"], "michael");
    assert_eq!(t["projects"][0]["topics"][0]["memory"]["limit"], 16384);
}

#[tokio::test]
async fn secrets_are_masked_on_write() {
    let e = env().await;
    let r = e
        .send(
            "PUT",
            "/api/vault",
            json!({"scope": "global", "name": "GITHUB_TOKEN", "value": "ghp_tres_secret"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let r = put(&e, "", "memory", "- jeton ghp_tres_secret\n").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let r = e.get("/api/memory/file?path=&file=memory").await;
    assert_eq!(r.body["content"], "- jeton ***\n");
}

#[test]
fn new_errors_have_their_status() {
    for (err, status, code) in [
        (ApiError::MemoryChanged, StatusCode::CONFLICT, "memory_changed"),
        (ApiError::TooLarge, StatusCode::PAYLOAD_TOO_LARGE, "too_large"),
        (ApiError::InvalidSkill, StatusCode::BAD_REQUEST, "invalid_skill"),
    ] {
        assert_eq!(err.to_string(), code);
        assert_eq!(err.into_response().status(), status);
    }
}

#[tokio::test]
async fn deleting_a_topic_removes_its_memory() {
    let e = env().await;
    let r = e.send("POST", "/api/projects", json!({"name": "Site"})).await;
    let pid = r.body["id"].as_i64().unwrap();
    let r = e
        .send("POST", &format!("/api/projects/{pid}/topics"), json!({"name": "Front"}))
        .await;
    let tid = r.body["id"].as_i64().unwrap();
    assert_eq!(put(&e, "site/front", "memory", "- a\n").await.status, StatusCode::OK);
    assert_eq!(put(&e, "site", "memory", "- b\n").await.status, StatusCode::OK);
    let mem = e.dir.path().join("data/workspace/memory");
    assert!(mem.join("site/front/memory.md").is_file());

    let r = e.send("DELETE", &format!("/api/topics/{tid}"), json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert!(!mem.join("site/front").exists());
    assert!(mem.join("site/memory.md").is_file());
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memory_revisions WHERE path LIKE 'site/front/%'")
        .fetch_one(e.state.db())
        .await
        .unwrap();
    assert_eq!(n, 0);
    let r = e.send("DELETE", &format!("/api/projects/{pid}"), json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert!(!mem.join("site").exists());
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memory_revisions WHERE path LIKE 'site/%'")
        .fetch_one(e.state.db())
        .await
        .unwrap();
    assert_eq!(n, 0);
}

// ---- Recall ----

async fn add_turn(e: &Env, sid: &str, turn: &str, origin: &str) {
    sqlx::query("INSERT INTO chat_turns (id, session_id, status, started_at, origin) VALUES (?, ?, 'completed', ?, ?)")
        .bind(turn)
        .bind(sid)
        .bind(now())
        .bind(origin)
        .execute(e.state.db())
        .await
        .unwrap();
}

async fn add_item(e: &Env, sid: &str, turn: &str, item: &str, kind: &str, text: &str) {
    let seq: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(seq), 0) + 1 FROM chat_items WHERE session_id = ?")
        .bind(sid)
        .fetch_one(e.state.db())
        .await
        .unwrap();
    sqlx::query("INSERT INTO chat_items (session_id, turn_id, item_id, seq, kind, data) VALUES (?, ?, ?, ?, ?, ?)")
        .bind(sid)
        .bind(turn)
        .bind(item)
        .bind(seq)
        .bind(kind)
        .bind(json!({"type": kind, "id": item, "text": text}).to_string())
        .execute(e.state.db())
        .await
        .unwrap();
}

async fn search(e: &Env, q: &str, scope: recall::Scope, topic_id: i64) -> Vec<recall::Hit> {
    let pid: i64 = sqlx::query_scalar("SELECT project_id FROM topics WHERE id = ?")
        .bind(topic_id)
        .fetch_one(e.state.db())
        .await
        .unwrap();
    recall::search(&e.state, q, scope, pid, topic_id, 8).await.unwrap()
}

#[tokio::test]
async fn recall_finds_user_turns_only() {
    let e = env().await;
    let inbox = inbox(&e).await;
    let a = new_session(&e, "auto").await;
    add_turn(&e, &a, "t1", "user").await;
    add_item(&e, &a, "t1", "ex-1", "userMessage", "Quel est le port de la prod ?").await;
    add_item(&e, &a, "t1", "i2", "agentMessage", "La production écoute sur 8095.").await;
    add_turn(&e, &a, "t2", "verify").await;
    add_item(&e, &a, "t2", "i3", "agentMessage", "Vérification : tortue verte").await;
    add_turn(&e, &a, "t3", "memory").await;
    add_item(&e, &a, "t3", "i4", "agentMessage", "Mémoire : girafe bleue").await;
    add_turn(&e, &a, "t4", "user").await;
    add_item(&e, &a, "t4", "i5", "agentMessage", "Un zèbre rose").await;
    sqlx::query("INSERT INTO chat_reverted (session_id, turn_id) VALUES (?, 't4')")
        .bind(&a)
        .execute(e.state.db())
        .await
        .unwrap();

    let h = search(&e, "port", recall::Scope::All, inbox).await;
    assert_eq!(h.len(), 1);
    assert!(h[0].from_user);
    let line = h[0].line();
    assert!(
        line.contains(&format!(
            "general/inbox · « Nouvelle session » (session {a}, n° 1) Michael : "
        )),
        "{line}"
    );
    assert!(line.contains("«port»"), "{line}");
    // Accents ignorés, débuts de mots.
    let h = search(&e, "production ecoute", recall::Scope::All, inbox).await;
    assert_eq!(h.len(), 1);
    assert!(!h[0].from_user);
    assert!(search(&e, "tortue", recall::Scope::All, inbox).await.is_empty());
    assert!(search(&e, "girafe", recall::Scope::All, inbox).await.is_empty());
    assert!(search(&e, "zèbre", recall::Scope::All, inbox).await.is_empty());
    // Repli en OU.
    let h = search(&e, "port licorne", recall::Scope::All, inbox).await;
    assert_eq!(h.len(), 1);
    assert!(search(&e, "x", recall::Scope::All, inbox).await.is_empty());

    // Portée : une session d'un autre topic, archivée.
    let r = e.send("POST", "/api/projects", json!({"name": "Autre"})).await;
    let pid = r.body["id"].as_i64().unwrap();
    let r = e
        .send("POST", &format!("/api/projects/{pid}/topics"), json!({"name": "T"}))
        .await;
    let other = r.body["id"].as_i64().unwrap();
    let r = e
        .send("POST", "/api/sessions", json!({"topic_id": other, "approval": "auto"}))
        .await;
    let b = r.body["id"].as_str().unwrap().to_string();
    add_turn(&e, &b, "u1", "user").await;
    add_item(&e, &b, "u1", "ex-9", "userMessage", "le port du staging est 8096").await;
    sqlx::query("UPDATE chat_sessions SET archived = 1 WHERE id = ?")
        .bind(&b)
        .execute(e.state.db())
        .await
        .unwrap();
    assert_eq!(search(&e, "port", recall::Scope::All, inbox).await.len(), 2);
    assert_eq!(search(&e, "port", recall::Scope::Topic, inbox).await.len(), 1);
    assert_eq!(search(&e, "port", recall::Scope::Project, inbox).await.len(), 1);
    assert_eq!(search(&e, "port", recall::Scope::Topic, other).await[0].session_id, b);

    // Lecture d'une session : mêmes exclusions, pagination.
    let m = recall::read_session(&e.state, &a, 1, 20).await.unwrap();
    assert_eq!(m.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![1, 2]);
    let m = recall::read_session(&e.state, &a, 2, 1).await.unwrap();
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].text, "La production écoute sur 8095.");

    // Suppression d'une session : l'index suit.
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM recall_docs")
        .fetch_one(e.state.db())
        .await
        .unwrap();
    assert_eq!(n, 6);
    sqlx::query("DELETE FROM chat_sessions WHERE id = ?")
        .bind(&b)
        .execute(e.state.db())
        .await
        .unwrap();
    assert_eq!(search(&e, "port", recall::Scope::All, inbox).await.len(), 1);
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM recall_fts")
        .fetch_one(e.state.db())
        .await
        .unwrap();
    assert_eq!(n, 5);
}

#[tokio::test]
async fn session_list_search_uses_the_index() {
    let e = env().await;
    let a = new_session(&e, "auto").await;
    add_turn(&e, &a, "t1", "user").await;
    add_item(&e, &a, "t1", "ex-1", "userMessage", "La vérification échoue").await;
    add_turn(&e, &a, "t2", "verify").await;
    add_item(&e, &a, "t2", "i2", "agentMessage", "hippopotame").await;
    let b = new_session(&e, "auto").await;
    e.send(
        "PATCH",
        &format!("/api/sessions/{b}"),
        json!({"title": "Réglage du proxy"}),
    )
    .await;

    let ids = |v: serde_json::Value| -> Vec<String> {
        v["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(ids(e.get("/api/sessions?q=verif").await.body), vec![a.clone()]);
    assert_eq!(ids(e.get("/api/sessions?q=echoue").await.body), vec![a.clone()]);
    assert_eq!(ids(e.get("/api/sessions?q=proxy").await.body), vec![b.clone()]);
    assert!(ids(e.get("/api/sessions?q=hippopotame").await.body).is_empty());
    // Sans mot valable : le titre seul (« ! » n'est dans aucun titre).
    let r = e.get("/api/sessions?q=%21").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert!(ids(r.body).is_empty());
    assert_eq!(ids(e.get("/api/sessions?q=%C3%A9").await.body), vec![b.clone()]);
}

#[tokio::test]
async fn migration_indexes_existing_messages() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    // Base en 0005 avec un message.
    {
        use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
        let opts = SqliteConnectOptions::new()
            .filename(data.join("enginex.db"))
            .create_if_missing(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal);
        let pool = SqlitePoolOptions::new().connect_with(opts).await.unwrap();
        let full = sqlx::migrate!("./migrations");
        let old = sqlx::migrate::Migrator {
            migrations: full.migrations.iter().filter(|m| m.version <= 5).cloned().collect(),
            ..full
        };
        old.run(&pool).await.unwrap();
        let t = now();
        sqlx::query("INSERT INTO projects (slug, name, created_at) VALUES ('general', 'Général', ?)")
            .bind(t)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO topics (project_id, slug, name, created_at) VALUES (1, 'inbox', 'Inbox', ?)")
            .bind(t)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO chat_sessions (id, topic_id, title, created_at, updated_at) VALUES ('s1', 1, 'Ancienne', ?, ?)",
        )
        .bind(t)
        .bind(t)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO chat_items (session_id, turn_id, item_id, seq, kind, data) \
             VALUES ('s1', NULL, 'ex-1', 1, 'userMessage', '{\"type\":\"userMessage\",\"text\":\"ancien message kiwi\"}')",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
    }
    let pool = enginex_server::db::open(&data).await.unwrap();
    assert!(data.join("enginex.db.pre-0006").is_file());
    let n: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM recall_fts f JOIN recall_docs d ON d.id = f.rowid WHERE recall_fts MATCH 'kiwi'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1);
    pool.close().await;
    // Réouverture : la copie n'est pas refaite.
    let before = std::fs::metadata(data.join("enginex.db.pre-0006"))
        .unwrap()
        .modified()
        .unwrap();
    let pool = enginex_server::db::open(&data).await.unwrap();
    pool.close().await;
    let after = std::fs::metadata(data.join("enginex.db.pre-0006"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(before, after);
}

// ---- Pont ----

/// Le tour du script `name` (de `turn/start` à `turn/completed`), renuméroté
/// en tour `n` (éléments `$item:<n><k>`).
fn turn_steps(name: &str, n: usize) -> Vec<Value> {
    let steps = script_with(name, |_| {});
    let start = step_pos(&steps, 0, is_expect("turn/start"));
    let end = step_pos(&steps, 0, is_send("turn/completed"));
    steps[start..=end]
        .iter()
        .map(|s| {
            let mut t = s.to_string().replace("$turn:1", &format!("$turn:{n}"));
            for k in 1..=9 {
                t = t.replace(&format!("$item:{k}\""), &format!("$item:{n}{k}\""));
            }
            serde_json::from_str(&t).unwrap()
        })
        .collect()
}

/// Premier lancement du script `text` (connexion, fil, premier tour).
fn first_segment() -> Vec<Value> {
    let steps = script_with("text", |_| {});
    let end = step_pos(&steps, 0, is_send("turn/completed"));
    steps[..=end].to_vec()
}

/// Tour `n` qui ne finit que sur `turn/interrupt`.
fn interruptible_turn(n: usize) -> Vec<Value> {
    let start = turn_steps("text", n)[0].clone();
    vec![
        start,
        json!({"send": {"method": "turn/started", "params": {"threadId": "$thread",
               "turn": {"id": format!("$turn:{n}"), "items": [], "status": "inProgress"}}}}),
        json!({"wait_interrupt": true}),
        json!({"send": {"method": "turn/completed", "params": {"threadId": "$thread",
               "turn": {"id": format!("$turn:{n}"), "items": [], "status": "interrupted", "error": null}}}}),
    ]
}

/// Textes d'entrée du `n`-ième `turn/start` reçu.
fn inputs(fake: &FakeAs, n: usize) -> Vec<String> {
    fake.received("turn/start")[n]["params"]["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|i| i["text"].as_str().map(str::to_string))
        .collect()
}

async fn seen(e: &Env, sid: &str) -> Option<String> {
    sqlx::query_scalar("SELECT memory_seen FROM chat_sessions WHERE id = ?")
        .bind(sid)
        .fetch_one(e.state.db())
        .await
        .unwrap()
}

fn block_sha(e: &Env) -> String {
    enginex_server::memory::block(&e.state, "general", "inbox").1
}

/// Clé interne de la sandbox d'une session, pour appeler `/mcp` comme elle.
async fn sandbox_key(e: &Env, sid: &str) -> String {
    let key = format!("ex_test{}", uuid::Uuid::new_v4().simple());
    // Remplace la clé de la sandbox si elle existe (le faux Codex ne s'en sert pas).
    sqlx::query(
        "INSERT INTO api_keys (name, hash, created_at, internal) VALUES (?, ?, ?, 1) \
         ON CONFLICT(name) DO UPDATE SET hash = excluded.hash",
    )
    .bind(format!("sandbox:{sid}"))
    .bind(Sha256::digest(key.as_bytes()).to_vec())
    .bind(now())
    .execute(e.state.db())
    .await
    .unwrap();
    key
}

async fn mem_add(e: &Env, key: &str, level: &str, text: &str) {
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                      "params": {"name": "mem_add", "arguments": {"level": level, "text": text}}});
    let req = axum::http::Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("host", "enginex:8090")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {key}"))
        .body(axum::body::Body::from(body.to_string()))
        .unwrap();
    let res = e.router.clone().oneshot(req).await.unwrap();
    let v: Value = serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(v["result"]["isError"], false, "{v}");
}

fn topic_file(e: &Env) -> std::path::PathBuf {
    e.dir.path().join("data/workspace/memory/general/inbox/memory.md")
}

#[tokio::test]
async fn block_is_sent_when_memory_changes() {
    let mut script = first_segment();
    for n in 2..=5 {
        let mut t = turn_steps("text", n);
        if n == 4 {
            // Compactage : Codex refait son message developer.
            let i = t.len() - 1;
            t.insert(i, json!({"sleep_ms": 300}));
            t.insert(
                i + 1,
                json!({"send": {"method": "item/completed", "params": {"threadId": "$thread", "turnId": "$turn:4",
                       "item": {"type": "contextCompaction", "id": "$item:49"}}}}),
            );
        }
        script.extend(t);
    }
    script.push(json!({"expect": "thread/revert", "result": {}}));
    script.extend(turn_steps("text", 6));
    let (e, fake) = env_sessions_cfg(&script, "", |_| {}).await;
    make_active_preset(&e).await;
    std::fs::create_dir_all(topic_file(&e).parent().unwrap()).unwrap();
    std::fs::write(topic_file(&e), "- fait A\n").unwrap();
    let sid = new_session(&e, "auto").await;
    let other = new_session(&e, "auto").await;
    let other_key = sandbox_key(&e, &other).await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    let done = |_: usize| |v: &Value| v["turn"]["status"] == "completed";

    turn(&e, &sid, "un").await;
    ev.until("turn_stored", done(1)).await;
    let start = &fake.received("thread/start")[0]["params"];
    let instr = start["developerInstructions"].as_str().unwrap();
    assert!(instr.contains("EngineX garde des notes en 3 niveaux (global, projet general, topic inbox)"));
    assert!(instr.contains("### Topic inbox : mémoire\n- fait A"), "{instr}");
    assert!(instr.starts_with(enginex_server::verify::DEVELOPER_INSTRUCTIONS));
    assert!(inputs(&fake, 0).iter().all(|t| !t.contains("<memoire_enginex>")));
    let sha1 = block_sha(&e);
    assert_eq!(seen(&e, &sid).await.as_deref(), Some(sha1.as_str()));

    // Mémoire modifiée par Michael : le bloc part avec le tour suivant.
    std::fs::write(topic_file(&e), "- fait A\n- fait B\n").unwrap();
    turn(&e, &sid, "deux").await;
    ev.until("turn_stored", done(2)).await;
    let t = inputs(&fake, 1);
    assert!(
        t[0].starts_with("<memoire_enginex>\n") && t[0].contains("- fait B"),
        "{t:?}"
    );
    assert!(t[0].ends_with("</memoire_enginex>\nCette version remplace la précédente."));
    assert_eq!(t[1], "deux");
    assert_eq!(seen(&e, &sid).await, Some(block_sha(&e)));

    // Écriture de la session elle-même : pas de renvoi. (Clé posée après le
    // démarrage de la sandbox, qui remplace la clé interne de la session.)
    let key = sandbox_key(&e, &sid).await;
    mem_add(&e, &key, "topic", "fait C").await;
    turn(&e, &sid, "trois").await;
    ev.until("turn_stored", done(3)).await;
    assert_eq!(inputs(&fake, 2), vec!["trois"]);

    // Écriture d'une autre session : renvoi ; puis compactage pendant ce tour.
    mem_add(&e, &other_key, "project", "fait D").await;
    turn(&e, &sid, "quatre").await;
    ev.until("turn_stored", done(4)).await;
    assert!(inputs(&fake, 3)[0].contains("fait D"));
    // Codex a repris les instructions de la connexion : le bloc d'alors.
    assert_eq!(seen(&e, &sid).await.as_deref(), Some(sha1.as_str()));
    turn(&e, &sid, "cinq").await;
    ev.until("turn_stored", done(5)).await;
    assert!(inputs(&fake, 4)[0].starts_with("<memoire_enginex>"));

    // Retour arrière avec la conversation : renvoi.
    sqlx::query("UPDATE chat_sessions SET pending_revert = 'x' WHERE id = ?")
        .bind(&sid)
        .execute(e.state.db())
        .await
        .unwrap();
    turn(&e, &sid, "six").await;
    ev.until("turn_stored", done(6)).await;
    assert!(inputs(&fake, 5)[0].starts_with("<memoire_enginex>"));
    assert!(fake.unexpected().is_empty(), "{:?}", fake.unexpected());
}

/// Script `resume` : segment 1 (premier lancement), segment 2 (relance).
fn resume_segments() -> (Vec<Value>, Vec<Value>) {
    let steps = script_with("resume", |_| {});
    let i = steps.iter().position(|s| s.get("launch").is_some()).unwrap();
    (steps[..i].to_vec(), steps[i + 1..].to_vec())
}

#[tokio::test]
async fn resume_carries_instructions_and_refused_resume_starts_a_thread() {
    let (seg1, seg2) = resume_segments();
    // Troisième lancement : reprise refusée, nouveau fil.
    let thread_start = seg1[step_pos(&seg1, 0, is_expect("thread/start"))].clone();
    let mut seg3: Vec<Value> = seg2.clone();
    let i = step_pos(&seg3, 0, is_expect("thread/resume"));
    seg3[i] = json!({"expect": "thread/resume", "error": {"code": -32600, "message": "no rollout found"}});
    seg3.insert(i + 1, thread_start);
    seg3.retain(|s| s["expect"] != "thread/turns/list");
    let mut script = seg1;
    script.push(json!({"launch": true}));
    script.extend(seg2);
    script.push(json!({"launch": true}));
    script.extend(seg3);
    let (e, fake) = env_sessions_cfg(&script, "", |_| {}).await;
    make_active_preset(&e).await;
    let sid = new_session(&e, "auto").await;
    let hub = enginex_server::bridge::hub(&e.state, &sid);
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    turn(&e, &sid, "un").await;
    ev.until("turn_stored", |_| true).await;
    hub.stop_conn(&e.state, "test").await;

    std::fs::create_dir_all(topic_file(&e).parent().unwrap()).unwrap();
    std::fs::write(topic_file(&e), "- port 8095\n").unwrap();
    turn(&e, &sid, "deux").await;
    ev.until("turn_stored", |_| true).await;
    let resume = &fake.received("thread/resume")[0]["params"];
    assert!(
        resume["developerInstructions"]
            .as_str()
            .unwrap()
            .contains("- port 8095"),
        "{resume}"
    );
    // Le message developer d'origine reste chez Codex : bloc joint au tour.
    assert!(inputs(&fake, 1)[0].contains("- port 8095"));
    hub.stop_conn(&e.state, "test").await;

    std::fs::write(topic_file(&e), "- port 8096\n").unwrap();
    turn(&e, &sid, "trois").await;
    ev.until("turn_stored", |_| true).await;
    let starts = fake.received("thread/start");
    assert_eq!(starts.len(), 2);
    assert!(
        starts[1]["params"]["developerInstructions"]
            .as_str()
            .unwrap()
            .contains("- port 8096")
    );
    assert_eq!(seen(&e, &sid).await, Some(block_sha(&e)));
    assert_eq!(inputs(&fake, 2), vec!["trois"]);
}

// ---- Tour de fin de session ----

async fn turns_of(e: &Env, sid: &str) -> Vec<(String, String)> {
    sqlx::query_as("SELECT origin, status FROM chat_turns WHERE session_id = ? ORDER BY rowid")
        .bind(sid)
        .fetch_all(e.state.db())
        .await
        .unwrap()
}

fn idle_cfg(c: &mut enginex_server::config::Config) {
    c.timings.sandbox.idle_override = Some(Duration::from_millis(400));
}

#[tokio::test]
async fn idle_session_runs_a_memory_turn_then_stops() {
    let mut script = first_segment();
    script.extend(turn_steps("text", 2));
    let (e, fake) = env_sessions_cfg(&script, "", idle_cfg).await;
    make_active_preset(&e).await;
    let sid = new_session(&e, "auto").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    turn(&e, &sid, "un").await;
    ev.until("turn_stored", |_| true).await;
    let m = ev.until("user_message", |v| v["item"]["origin"] == "memory").await;
    assert_eq!(m["item"]["text"], enginex_server::memory::END_PROMPT);
    let t = ev.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["origin"], "memory");
    let s = ev.until("sandbox", |v| v["state"] == "stopped").await;
    assert_eq!(s["detail"], "inactivité");
    assert_eq!(
        turns_of(&e, &sid).await,
        vec![
            ("user".into(), "completed".into()),
            ("memory".into(), "completed".into())
        ]
    );
    let start = &fake.received("turn/start")[1]["params"];
    assert_eq!(start["approvalPolicy"], "untrusted", "{start}");
    assert_eq!(inputs(&fake, 1).last().unwrap(), enginex_server::memory::END_PROMPT);
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM verifications")
        .fetch_one(e.state.db())
        .await
        .unwrap();
    assert_eq!(n, 0);
    // Une seule fois : pas de nouveau tour de Michael depuis.
    assert!(!enginex_server::memory::should_close_with_turn(&e.state, &sid).await);
    // Le message de fin est replié dans le fil (origine gardée).
    let items = e.get(&format!("/api/sessions/{sid}")).await.body["items"].clone();
    assert!(
        items
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["data"]["origin"] == "memory"),
        "{items}"
    );
    assert!(fake.unexpected().is_empty(), "{:?}", fake.unexpected());
}

#[tokio::test]
async fn memory_auto_off_stops_directly() {
    let (e, fake) = env_sessions_cfg(&first_segment(), "", idle_cfg).await;
    make_active_preset(&e).await;
    let r = e
        .send("PUT", "/api/settings/sessions", json!({"memory_auto": false}))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(e.get("/api/settings/sessions").await.body["memory_auto"], false);
    let sid = new_session(&e, "auto").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    turn(&e, &sid, "un").await;
    ev.until("turn_stored", |_| true).await;
    ev.until("sandbox", |v| v["state"] == "stopped").await;
    assert_eq!(turns_of(&e, &sid).await.len(), 1);
    assert!(fake.unexpected().is_empty(), "{:?}", fake.unexpected());
}

#[tokio::test]
async fn requests_during_memory_turn_are_declined_by_the_bridge() {
    let mut script = first_segment();
    let mut t = turn_steps("text", 2);
    let req = |id: &str, method: &str| {
        json!({"send": {"id": id, "method": method, "params": {"threadId": "$thread", "turnId": "$turn:2",
               "itemId": "$item:29", "command": "rm -rf /", "questions": []}}})
    };
    t.insert(1, req("$req:1", "item/commandExecution/requestApproval"));
    t.insert(2, json!({"expect_reply": "$req:1"}));
    t.insert(3, req("$req:2", "item/tool/requestUserInput"));
    t.insert(4, json!({"expect_reply": "$req:2"}));
    script.extend(t);
    let (e, fake) = env_sessions_cfg(&script, "", idle_cfg).await;
    make_active_preset(&e).await;
    let sid = new_session(&e, "prudent").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    turn(&e, &sid, "un").await;
    ev.until("turn_stored", |_| true).await;
    ev.until("turn_stored", |v| v["turn"]["origin"] == "memory").await;
    let replies: Vec<Value> = fake.log().into_iter().filter_map(|l| l.get("reply").cloned()).collect();
    assert_eq!(replies.len(), 2, "{replies:?}");
    assert_eq!(replies[0]["result"], json!({"decision": "decline"}));
    assert_eq!(replies[1]["result"], json!({"answers": {}}));
    assert!(!ev.names().iter().any(|n| n == "request"), "{:?}", ev.names());
}

#[tokio::test]
async fn user_message_preempts_memory_turn() {
    let mut script = first_segment();
    script.extend(interruptible_turn(2));
    script.extend(turn_steps("text", 3));
    script.extend(turn_steps("text", 4));
    let (e, fake) = env_sessions_cfg(&script, "", idle_cfg).await;
    make_active_preset(&e).await;
    let sid = new_session(&e, "auto").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    turn(&e, &sid, "un").await;
    ev.until("turn_stored", |_| true).await;
    ev.until("user_message", |v| v["item"]["origin"] == "memory").await;
    let r = turn(&e, &sid, "encore").await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text);
    ev.until("turn_stored", |v| v["turn"]["origin"] == "memory").await;
    let t = ev.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["origin"], "user");
    assert_eq!(t["turn"]["status"], "completed");
    assert!(
        !ev.seen.iter().any(|(n, v)| n == "sandbox" && v["state"] == "stopped"),
        "{:?}",
        ev.names()
    );
    let origins: Vec<String> = turns_of(&e, &sid).await.into_iter().map(|t| t.0).collect();
    assert_eq!(&origins[..3], ["user", "memory", "user"]);
    assert_eq!(turns_of(&e, &sid).await[1].1, "interrupted");
    // Nouvelle inactivité : un tour de mémoire de nouveau, puis l'arrêt.
    ev.until("sandbox", |v| v["state"] == "stopped").await;
    assert_eq!(turns_of(&e, &sid).await.len(), 4);
    assert!(fake.unexpected().is_empty(), "{:?}", fake.unexpected());
}

#[tokio::test]
async fn memory_turn_times_out() {
    let mut script = first_segment();
    script.extend(interruptible_turn(2));
    let (e, fake) = env_sessions_cfg(&script, "", |c| {
        idle_cfg(c);
        c.timings.sandbox.memory_turn_timeout = Duration::from_secs(1);
    })
    .await;
    make_active_preset(&e).await;
    let sid = new_session(&e, "auto").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    turn(&e, &sid, "un").await;
    ev.until("turn_stored", |_| true).await;
    let t = ev.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["status"], "interrupted");
    let s = ev.until("sandbox", |v| v["state"] == "stopped").await;
    assert_eq!(s["detail"], "inactivité");
    assert_eq!(fake.received("turn/interrupt").len(), 1);
}

#[tokio::test]
async fn make_room_interrupts_a_memory_turn() {
    let mut script = first_segment();
    script.extend(interruptible_turn(2));
    script.push(json!({"launch": true}));
    script.extend(first_segment());
    script.extend(turn_steps("text", 2));
    let (e, fake) = env_sessions_cfg(&script, "", idle_cfg).await;
    make_active_preset(&e).await;
    let r = e.send("PUT", "/api/settings/sessions", json!({"max": 1})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let a = new_session(&e, "auto").await;
    let b = new_session(&e, "auto").await;
    let mut ea = sse(&e, &format!("/api/sessions/{a}/events")).await;
    let mut eb = sse(&e, &format!("/api/sessions/{b}/events")).await;
    turn(&e, &a, "un").await;
    ea.until("turn_stored", |_| true).await;
    ea.until("user_message", |v| v["item"]["origin"] == "memory").await;
    let r = turn(&e, &b, "à moi").await;
    assert_eq!(r.status, StatusCode::ACCEPTED, "{}", r.text);
    let s = ea.until("sandbox", |v| v["state"] == "stopped").await;
    assert_eq!(s["detail"], "place libérée pour une autre session");
    let t = eb.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["status"], "completed", "{t}");
    assert_eq!(turns_of(&e, &a).await[1], ("memory".into(), "interrupted".into()));
    assert!(fake.unexpected().is_empty(), "{:?}", fake.unexpected());
}

#[tokio::test]
async fn stop_if_idle_never_kills_an_open_turn() {
    let mut script = first_segment();
    let i = step_pos(&script, 0, is_send("turn/completed"));
    script.insert(i, json!({"sleep_ms": 500}));
    let (e, fake) = env_sessions_cfg(&script, "", |_| {}).await;
    make_active_preset(&e).await;
    let sid = new_session(&e, "auto").await;
    let hub = enginex_server::bridge::hub(&e.state, &sid);
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    let r = turn(&e, &sid, "un").await;
    assert_eq!(r.status, StatusCode::ACCEPTED);
    // Le tour est ouvert dès la réponse : l'arrêt pour inactivité s'abstient.
    assert!(!hub.stop_if_idle(&e.state, Duration::ZERO, "inactivité").await);
    let t = ev.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["status"], "completed");
    assert!(hub.stop_if_idle(&e.state, Duration::ZERO, "inactivité").await);
    assert!(!hub.has_conn());
    assert!(fake.unexpected().is_empty(), "{:?}", fake.unexpected());
}
