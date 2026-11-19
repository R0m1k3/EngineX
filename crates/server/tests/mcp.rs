//! Serveur MCP intégré : clés, protocole, outils mémoire et recall.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use common::{Env, env, new_key, new_session};
use enginex_server::state::now;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

/// Clé interne de la sandbox d'une session (comme `sandboxes::start`).
async fn sandbox_key(e: &Env, sid: &str) -> String {
    let key = format!("ex_test{}", uuid::Uuid::new_v4().simple());
    sqlx::query("INSERT INTO api_keys (name, hash, created_at, internal) VALUES (?, ?, ?, 1)")
        .bind(format!("sandbox:{sid}"))
        .bind(Sha256::digest(key.as_bytes()).to_vec())
        .bind(now())
        .execute(e.state.db())
        .await
        .unwrap();
    key
}

struct Out {
    status: StatusCode,
    body: Value,
    headers: axum::http::HeaderMap,
}

async fn raw(e: &Env, method: &str, key: Option<&str>, body: Option<Value>) -> Out {
    let mut b = Request::builder()
        .method(method)
        .uri("/mcp")
        .header(header::HOST, "enginex:8090")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(k) = key {
        b = b.header(header::AUTHORIZATION, format!("Bearer {k}"));
    }
    let req = b
        .body(body.map_or_else(Body::empty, |v| Body::from(v.to_string())))
        .unwrap();
    let res = e.router.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    Out {
        status,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        headers,
    }
}

async fn rpc(e: &Env, key: &str, method: &str, params: Value) -> Value {
    let o = raw(
        e,
        "POST",
        Some(key),
        Some(json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params})),
    )
    .await;
    assert_eq!(o.status, StatusCode::OK, "{}", o.body);
    assert_eq!(o.body["id"], 7);
    o.body
}

/// Appel d'outil : `(texte, isError)`.
async fn tool(e: &Env, key: &str, name: &str, args: Value) -> (String, bool) {
    let v = rpc(e, key, "tools/call", json!({"name": name, "arguments": args})).await;
    assert!(v.get("error").is_none(), "{v}");
    (
        v["result"]["content"][0]["text"].as_str().unwrap().to_string(),
        v["result"]["isError"].as_bool().unwrap(),
    )
}

fn topic_memory(e: &Env) -> String {
    std::fs::read_to_string(e.dir.path().join("data/workspace/memory/general/inbox/memory.md")).unwrap_or_default()
}

async fn memory_seen(e: &Env, sid: &str) -> Option<String> {
    sqlx::query_scalar("SELECT memory_seen FROM chat_sessions WHERE id = ?")
        .bind(sid)
        .fetch_one(e.state.db())
        .await
        .unwrap()
}

#[tokio::test]
async fn keys_and_protocol() {
    let e = env().await;
    let sid = new_session(&e, "auto").await;
    let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                      "params": {"protocolVersion": "2025-03-26", "capabilities": {}}});
    assert_eq!(
        raw(&e, "POST", None, Some(init.clone())).await.status,
        StatusCode::UNAUTHORIZED
    );
    let o = raw(&e, "POST", Some("ex_inconnue"), Some(init.clone())).await;
    assert_eq!(o.status, StatusCode::UNAUTHORIZED);
    assert_eq!(o.body["error"]["code"], "invalid_api_key");
    let user = new_key(&e, "portable").await;
    assert_eq!(
        raw(&e, "POST", Some(&user), Some(init.clone())).await.status,
        StatusCode::FORBIDDEN
    );

    let key = sandbox_key(&e, &sid).await;
    let o = raw(&e, "POST", Some(&key), Some(init)).await;
    assert_eq!(o.status, StatusCode::OK);
    assert_eq!(o.body["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(o.body["result"]["serverInfo"]["name"], "enginex");
    assert!(o.headers.contains_key("mcp-session-id"));
    let v = rpc(&e, &key, "initialize", json!({"protocolVersion": "2099-01-01"})).await;
    assert_eq!(v["result"]["protocolVersion"], "2025-06-18");

    let o = raw(
        &e,
        "POST",
        Some(&key),
        Some(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})),
    )
    .await;
    assert_eq!(o.status, StatusCode::ACCEPTED);
    assert_eq!(
        raw(&e, "GET", Some(&key), None).await.status,
        StatusCode::METHOD_NOT_ALLOWED
    );
    assert_eq!(raw(&e, "DELETE", Some(&key), None).await.status, StatusCode::OK);

    let v = rpc(&e, &key, "tools/list", json!({})).await;
    let names: Vec<&str> = v["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "mem_read",
            "mem_add",
            "mem_replace",
            "mem_write",
            "recall",
            "session_read"
        ]
    );
    assert_eq!(rpc(&e, &key, "ping", json!({})).await["result"], json!({}));
    assert_eq!(
        rpc(&e, &key, "resources/list", json!({})).await["error"]["code"],
        -32601
    );
    let v = rpc(&e, &key, "tools/call", json!({"name": "rm_rf", "arguments": {}})).await;
    assert_eq!(v["error"]["code"], -32602);
}

#[tokio::test]
async fn mem_add_replace_and_limits() {
    let e = env().await;
    let sid = new_session(&e, "auto").await;
    let key = sandbox_key(&e, &sid).await;

    let (t, err) = tool(
        &e,
        &key,
        "mem_add",
        json!({"level": "topic", "text": "port de prod 8095"}),
    )
    .await;
    assert!(!err, "{t}");
    let mem = topic_memory(&e);
    let line = mem.lines().next().unwrap().to_string();
    assert!(
        line.starts_with("- 20") && line.ends_with(" : port de prod 8095"),
        "{mem}"
    );
    let (author,): (String,) =
        sqlx::query_as("SELECT author FROM memory_revisions WHERE path = 'general/inbox/memory.md' ORDER BY id DESC")
            .fetch_one(e.state.db())
            .await
            .unwrap();
    assert_eq!(author, format!("session:{sid}"));
    let (t, err) = tool(
        &e,
        &key,
        "mem_add",
        json!({"level": "topic", "text": "port de prod 8095"}),
    )
    .await;
    assert_eq!((t.as_str(), err), ("déjà noté", false));
    let (t, err) = tool(&e, &key, "mem_add", json!({"level": "topic", "text": "a\nb"})).await;
    assert!(err && t.contains("une ligne"), "{t}");

    // mem_replace : unique, absent, en double.
    let (t, err) = tool(
        &e,
        &key,
        "mem_replace",
        json!({"level": "topic", "old": "8095", "new": "8096"}),
    )
    .await;
    assert!(!err, "{t}");
    assert!(topic_memory(&e).contains("port de prod 8096"));
    let (t, err) = tool(
        &e,
        &key,
        "mem_replace",
        json!({"level": "topic", "old": "licorne", "new": "x"}),
    )
    .await;
    assert!(err && t.contains("introuvable"), "{t}");
    tool(
        &e,
        &key,
        "mem_add",
        json!({"level": "topic", "text": "port de test 8096"}),
    )
    .await;
    let (t, err) = tool(
        &e,
        &key,
        "mem_replace",
        json!({"level": "topic", "old": "8096", "new": "x"}),
    )
    .await;
    assert!(err && t.starts_with("2 occurrences"), "{t}");
    let (_, err) = tool(
        &e,
        &key,
        "mem_replace",
        json!({"level": "topic", "old": "port de test 8096", "new": ""}),
    )
    .await;
    assert!(!err);
    assert_eq!(topic_memory(&e).lines().count(), 1);

    // Au-delà de 12 Kio : avertissement ; au-delà de 16 Kio : refus.
    let filler: String = (0..130)
        .map(|i| format!("- 2026-01-01 : remplissage {i} {}\n", "x".repeat(80)))
        .collect();
    std::fs::write(
        e.dir.path().join("data/workspace/memory/general/inbox/memory.md"),
        &filler,
    )
    .unwrap();
    let (t, err) = tool(&e, &key, "mem_add", json!({"level": "topic", "text": "encore un"})).await;
    assert!(!err && t.contains("Kio sur 16 ; compacte-le bientôt"), "{t}");
    let big: String = (0..60)
        .map(|i| format!("- 2026-01-01 : bourrage {i} {}\n", "y".repeat(80)))
        .collect();
    std::fs::write(
        e.dir.path().join("data/workspace/memory/general/inbox/memory.md"),
        format!("{filler}{big}"),
    )
    .unwrap();
    let (t, err) = tool(&e, &key, "mem_add", json!({"level": "topic", "text": "de trop"})).await;
    assert!(err && t.contains("dépasse 16 Kio"), "{t}");

    // Les instructions se lisent, aucun outil ne les écrit.
    std::fs::write(e.dir.path().join("data/workspace/memory/instructions.md"), "Sois bref.").unwrap();
    let (t, _) = tool(&e, &key, "mem_read", json!({"level": "global", "file": "instructions"})).await;
    assert_eq!(t, "Sois bref.");
    let (t, _) = tool(&e, &key, "mem_read", json!({"level": "project"})).await;
    assert_eq!(t, "(vide)");
    let v = rpc(
        &e,
        &key,
        "tools/call",
        json!({"name": "mem_write", "arguments": {"level": "global", "file": "instructions", "content": "x"}}),
    )
    .await;
    assert_eq!(v["error"]["code"], -32602);
}

#[tokio::test]
async fn mem_write_requires_a_fresh_read() {
    let e = env().await;
    let a = new_session(&e, "auto").await;
    let b = new_session(&e, "auto").await;
    let ka = sandbox_key(&e, &a).await;
    let kb = sandbox_key(&e, &b).await;
    let w = |c: &str| json!({"level": "topic", "content": c});

    let (t, err) = tool(&e, &ka, "mem_write", w("- tout\n")).await;
    assert!(err && t.contains("lis d'abord"), "{t}");
    tool(&e, &ka, "mem_read", json!({"level": "topic"})).await;
    let (t, err) = tool(&e, &ka, "mem_write", w("- un\n")).await;
    assert!(!err, "{t}");
    assert_eq!(topic_memory(&e), "- un\n");
    // Une autre session écrit : la lecture de A est périmée.
    tool(&e, &kb, "mem_add", json!({"level": "topic", "text": "fait de B"})).await;
    let (t, err) = tool(&e, &ka, "mem_write", w("- deux\n")).await;
    assert!(err && t.contains("a changé depuis ta lecture"), "{t}");
    assert!(topic_memory(&e).contains("fait de B"));
    // Relecture, puis ajout de A lui-même : l'écriture complète reste permise.
    tool(&e, &ka, "mem_read", json!({"level": "topic"})).await;
    tool(&e, &ka, "mem_add", json!({"level": "topic", "text": "fait de A"})).await;
    let (t, err) = tool(&e, &ka, "mem_write", w("- compacté\n")).await;
    assert!(!err, "{t}");
    assert_eq!(topic_memory(&e), "- compacté\n");
}

#[tokio::test]
async fn memory_seen_follows_own_writes() {
    let e = env().await;
    let a = new_session(&e, "auto").await;
    let b = new_session(&e, "auto").await;
    let ka = sandbox_key(&e, &a).await;
    let kb = sandbox_key(&e, &b).await;
    // A a reçu le bloc vide (thread/start).
    let (_, empty) = enginex_server::memory::block(&e.state, "general", "inbox");
    sqlx::query("UPDATE chat_sessions SET memory_seen = ? WHERE id = ?")
        .bind(&empty)
        .bind(&a)
        .execute(e.state.db())
        .await
        .unwrap();
    tool(&e, &ka, "mem_add", json!({"level": "topic", "text": "fait un"})).await;
    let (_, now_sha) = enginex_server::memory::block(&e.state, "general", "inbox");
    assert_eq!(memory_seen(&e, &a).await.as_deref(), Some(now_sha.as_str()));
    // Une autre session écrit : A n'est plus à jour, et ne le redevient pas
    // par sa propre écriture suivante.
    tool(&e, &kb, "mem_add", json!({"level": "project", "text": "fait de B"})).await;
    tool(&e, &ka, "mem_add", json!({"level": "topic", "text": "fait deux"})).await;
    assert_eq!(memory_seen(&e, &a).await.as_deref(), Some(now_sha.as_str()));
    let (_, latest) = enginex_server::memory::block(&e.state, "general", "inbox");
    assert_ne!(latest, now_sha);
}

#[tokio::test]
async fn recall_and_session_read() {
    let e = env().await;
    let a = new_session(&e, "auto").await;
    let ka = sandbox_key(&e, &a).await;
    sqlx::query(
        "INSERT INTO chat_turns (id, session_id, status, started_at, origin) VALUES ('t1', ?, 'completed', ?, 'user')",
    )
    .bind(&a)
    .bind(now())
    .execute(e.state.db())
    .await
    .unwrap();
    for (i, (kind, text)) in [
        ("userMessage", "Où tourne le proxy Caddy ?"),
        ("agentMessage", "Le proxy Caddy tourne sur l'Unraid."),
        ("userMessage", "Et le port ?"),
    ]
    .iter()
    .enumerate()
    {
        sqlx::query(
            "INSERT INTO chat_items (session_id, turn_id, item_id, seq, kind, data) VALUES (?, 't1', ?, ?, ?, ?)",
        )
        .bind(&a)
        .bind(format!("i{i}"))
        .bind(i as i64 + 1)
        .bind(kind)
        .bind(json!({"type": kind, "text": text}).to_string())
        .execute(e.state.db())
        .await
        .unwrap();
    }
    let (t, err) = tool(&e, &ka, "recall", json!({"query": "caddy", "scope": "topic"})).await;
    assert!(!err, "{t}");
    let lines: Vec<&str> = t.lines().collect();
    assert_eq!(lines.len(), 2, "{t}");
    let date = enginex_server::memory::ymd(now());
    assert!(
        t.contains(&format!(
            "[{date}] general/inbox · « Nouvelle session » (session {a}, n° 1) Michael : "
        )),
        "{t}"
    );
    assert!(t.contains(&format!("(session {a}, n° 2) agent : ")), "{t}");
    let (t, _) = tool(&e, &ka, "recall", json!({"query": "licorne"})).await;
    assert_eq!(t, "aucun résultat");

    let (t, err) = tool(&e, &ka, "session_read", json!({"session_id": a, "limit": 2})).await;
    assert!(!err, "{t}");
    assert!(t.contains("n° 1 Michael : Où tourne le proxy Caddy ?"), "{t}");
    assert!(t.contains("n° 2 agent : "), "{t}");
    assert!(t.ends_with("(suite : from_seq = 3)"), "{t}");
    let (t, _) = tool(&e, &ka, "session_read", json!({"session_id": a, "from_seq": 3})).await;
    assert!(t.contains("n° 3 Michael : Et le port ?") && !t.contains("suite"), "{t}");
    let (t, err) = tool(&e, &ka, "session_read", json!({"session_id": "nope"})).await;
    assert!(err && t.contains("inconnue"), "{t}");
}
