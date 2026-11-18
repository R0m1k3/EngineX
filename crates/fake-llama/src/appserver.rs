//! Faux `codex app-server` : même argv que le vrai
//! (`app-server --listen ws://127.0.0.1:P --ws-auth capability-token
//! --ws-token-file F`), `GET /readyz` et un WebSocket JSON-RPC qui joue un
//! script :
//! - `FAKE_APPSERVER_SCRIPT` : fichier JSONL, une étape par ligne
//!   (`expect`/`result`, `send`, `expect_reply`, `sleep_ms`, `close`,
//!   `wait_interrupt`, `launch`) ; repères `$thread`, `$turn:N`, `$item:N`,
//!   `$req:N`, `$now`, `$cwd`, `$home` remplacés à l'envoi ;
//! - `FAKE_APPSERVER_STATE` : compteur de lancements (segment joué) ;
//! - `FAKE_APPSERVER_LOG` : chaque message reçu (`received`), chaque réponse à
//!   une requête du serveur (`reply`) et chaque message inattendu
//!   (`unexpected`, une requête inattendue reçoit -32601).

use std::{sync::Arc, time::Duration};

use axum::{
    Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use enginex_codex::record::substitute;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::{append_line, arg};

struct Fake {
    token: String,
    steps: Vec<Value>,
    log: Option<String>,
    cwd: String,
    home: String,
}

pub async fn run(args: &[String]) {
    let listen = arg(args, &["--listen"]).expect("--listen");
    let addr = listen.strip_prefix("ws://").expect("--listen ws://…").to_string();
    let token_file = arg(args, &["--ws-token-file"]).expect("--ws-token-file");
    let token = std::fs::read_to_string(token_file)
        .expect("jeton illisible")
        .trim()
        .to_string();
    let steps = segment();
    let fake = Arc::new(Fake {
        token,
        steps,
        log: std::env::var("FAKE_APPSERVER_LOG").ok(),
        cwd: std::env::current_dir()
            .map(|d| d.display().to_string())
            .unwrap_or_default(),
        home: std::env::var("CODEX_HOME").unwrap_or_default(),
    });
    let app = Router::new()
        .route("/readyz", get(|| async { "ok" }))
        .fallback(ws)
        .with_state(fake);
    let l = tokio::net::TcpListener::bind(&addr).await.expect("port");
    eprintln!("app-server: listening on ws://{addr}");
    axum::serve(l, app).await.expect("serve");
}

/// Segment du script pour ce lancement (compteur dans `FAKE_APPSERVER_STATE`).
fn segment() -> Vec<Value> {
    let Ok(path) = std::env::var("FAKE_APPSERVER_SCRIPT") else {
        return Vec::new();
    };
    let raw = std::fs::read_to_string(&path).expect("FAKE_APPSERVER_SCRIPT illisible");
    let mut segments: Vec<Vec<Value>> = vec![Vec::new()];
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let v: Value = serde_json::from_str(line).expect("étape invalide");
        if v.get("launch").is_some() {
            segments.push(Vec::new());
        } else {
            segments.last_mut().expect("segment").push(v);
        }
    }
    let rank = match std::env::var("FAKE_APPSERVER_STATE") {
        Ok(state) => {
            let n: usize = std::fs::read_to_string(&state)
                .ok()
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(0);
            let _ = std::fs::write(&state, (n + 1).to_string());
            n
        }
        Err(_) => 0,
    };
    segments.into_iter().nth(rank).unwrap_or_default()
}

async fn ws(
    State(fake): State<Arc<Fake>>,
    headers: HeaderMap,
    up: Result<WebSocketUpgrade, axum::extract::ws::rejection::WebSocketUpgradeRejection>,
) -> Response {
    let ok = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|t| t.trim() == fake.token);
    if !ok {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(up) = up else {
        return StatusCode::NOT_FOUND.into_response();
    };
    up.on_upgrade(move |socket| play(socket, fake))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

async fn play(socket: WebSocket, fake: Arc<Fake>) {
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    let log = fake.log.clone();
    tokio::spawn(async move {
        while let Some(Ok(m)) = stream.next().await {
            let text = match m {
                Message::Text(t) => t.to_string(),
                Message::Close(_) => break,
                _ => continue,
            };
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                if let Some(l) = &log {
                    append_line(l, &json!({"received": v}).to_string());
                }
                if tx.send(v).is_err() {
                    break;
                }
            }
        }
    });
    let sub = |v: &Value| substitute(v, &fake.cwd, &fake.home, now_ms());
    let write = |v: Value| Message::Text(v.to_string().into());
    let note = |kind: &str, v: &Value| {
        if let Some(l) = &fake.log {
            append_line(l, &json!({ kind: v }).to_string());
        }
    };
    for step in &fake.steps {
        if let Some(ms) = step.get("sleep_ms").and_then(Value::as_u64) {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        } else if step.get("close").is_some() {
            let _ = sink.send(Message::Close(None)).await;
            return;
        } else if let Some(v) = step.get("send") {
            if sink.send(write(sub(v))).await.is_err() {
                return;
            }
        } else if let Some(want) = step.get("expect_reply") {
            let want = sub(want);
            loop {
                let Some(m) = rx.recv().await else { return };
                if m.get("method").is_none() && m.get("id") == Some(&want) {
                    note("reply", &m);
                    break;
                }
                if !unexpected(&mut sink, &m, &note).await {
                    return;
                }
            }
        } else {
            let (method, result) = if step.get("wait_interrupt").is_some() {
                ("turn/interrupt".to_string(), json!({}))
            } else {
                (
                    step.get("expect").and_then(Value::as_str).unwrap_or("").to_string(),
                    step.get("result").cloned().unwrap_or(json!({})),
                )
            };
            loop {
                let Some(m) = rx.recv().await else { return };
                if m.get("method").and_then(Value::as_str) == Some(&method) {
                    if let Some(id) = m.get("id") {
                        let reply = json!({"id": id, "result": sub(&result)});
                        if sink.send(write(reply)).await.is_err() {
                            return;
                        }
                    }
                    break;
                }
                if !unexpected(&mut sink, &m, &note).await {
                    return;
                }
            }
        }
    }
    // Fin du script : toute requête reçoit -32601.
    while let Some(m) = rx.recv().await {
        if !unexpected(&mut sink, &m, &note).await {
            return;
        }
    }
}

async fn unexpected(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    m: &Value,
    note: &impl Fn(&str, &Value),
) -> bool {
    note("unexpected", m);
    if let (Some(id), Some(_)) = (m.get("id"), m.get("method")) {
        let e = json!({"id": id, "error": {"code": -32601, "message": "unexpected request"}});
        return sink.send(Message::Text(e.to_string().into())).await.is_ok();
    }
    true
}
