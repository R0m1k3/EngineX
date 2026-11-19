//! Mémoire (API, versions, suppression, migration) et recall (FTS, filtres).

mod common;

use axum::{http::StatusCode, response::IntoResponse};
use common::{Env, env, inbox, new_session};
use enginex_server::{error::ApiError, recall, state::now};
use serde_json::json;

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
    assert_eq!(e.get("/api/memory/revisions/999999").await.status, StatusCode::NOT_FOUND);

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
    sqlx::query(
        "INSERT INTO chat_turns (id, session_id, status, started_at, origin) VALUES (?, ?, 'completed', ?, ?)",
    )
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
        line.contains(&format!("general/inbox · « Nouvelle session » (session {a}, n° 1) Michael : ")),
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
    e.send("PATCH", &format!("/api/sessions/{b}"), json!({"title": "Réglage du proxy"}))
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
    let before = std::fs::metadata(data.join("enginex.db.pre-0006")).unwrap().modified().unwrap();
    let pool = enginex_server::db::open(&data).await.unwrap();
    pool.close().await;
    let after = std::fs::metadata(data.join("enginex.db.pre-0006")).unwrap().modified().unwrap();
    assert_eq!(before, after);
}
