//! Client contre le faux app-server (`cargo build -p enginex-fake-llama`
//! d'abord) : réponse, délai dépassé, requête du serveur, connexion coupée,
//! jeton refusé.

use std::{path::PathBuf, process::Stdio, time::Duration};

use enginex_codex::{Error, Incoming, connect};
use serde_json::{Value, json};

fn fake_bin() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.ancestors()
        .map(|d| d.join("fake-llama-server"))
        .find(|b| b.is_file())
        .unwrap_or_else(|| panic!("faux serveur absent : lancer `cargo build -p enginex-fake-llama`"))
}

struct Fake {
    _child: tokio::process::Child,
    url: String,
    log: PathBuf,
    _dir: tempfile::TempDir,
}

async fn fake(script: &[Value]) -> Fake {
    let dir = tempfile::tempdir().unwrap();
    let s = dir.path().join("script.jsonl");
    let lines: Vec<String> = script.iter().map(Value::to_string).collect();
    std::fs::write(&s, lines.join("\n")).unwrap();
    std::fs::write(dir.path().join("token"), "secret-token\n").unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let log = dir.path().join("log.jsonl");
    let child = tokio::process::Command::new(fake_bin())
        .args(["app-server", "--listen", &format!("ws://127.0.0.1:{port}")])
        .args(["--ws-auth", "capability-token", "--ws-token-file"])
        .arg(dir.path().join("token"))
        .env("FAKE_APPSERVER_SCRIPT", &s)
        .env("FAKE_APPSERVER_LOG", &log)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let ready = format!("http://127.0.0.1:{port}/readyz");
    for _ in 0..100 {
        if reqwest_like(&ready).await {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Fake {
        _child: child,
        url: format!("ws://127.0.0.1:{port}"),
        log,
        _dir: dir,
    }
}

/// GET minimal sans dépendance HTTP : la connexion TCP suffit à dire « prêt ».
async fn reqwest_like(url: &str) -> bool {
    let addr = url.trim_start_matches("http://").split('/').next().unwrap().to_string();
    tokio::net::TcpStream::connect(addr).await.is_ok()
}

fn log_lines(f: &Fake) -> Vec<Value> {
    std::fs::read_to_string(&f.log)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[tokio::test]
async fn request_gets_its_result() {
    let f = fake(&[
        json!({"expect": "initialize", "result": {"userAgent": "fake"}}),
        json!({"expect": "initialized"}),
        json!({"expect": "thread/start", "result": {"thread": {"id": "$thread"}}}),
        json!({"send": {"method": "thread/started", "params": {"thread": {"id": "$thread"}}}}),
    ])
    .await;
    let (c, mut rx) = connect(&f.url, "secret-token", Duration::from_secs(5), None)
        .await
        .unwrap();
    let init = c.initialize("0.1").await.unwrap();
    assert_eq!(init["userAgent"], "fake");
    let r = c.request("thread/start", json!({"cwd": "/tmp"})).await.unwrap();
    assert_eq!(r["thread"]["id"], "thread-1");
    match rx.recv().await.unwrap() {
        Incoming::Notification { method, params } => {
            assert_eq!(method, "thread/started");
            assert_eq!(params["thread"]["id"], "thread-1");
        }
        other => panic!("{other:?}"),
    }
    assert!(!log_lines(&f).iter().any(|l| l.get("unexpected").is_some()));
}

#[tokio::test]
async fn unexpected_request_is_an_rpc_error() {
    let f = fake(&[]).await;
    let (c, _rx) = connect(&f.url, "secret-token", Duration::from_secs(5), None)
        .await
        .unwrap();
    match c.request("thread/fork", json!({})).await {
        Err(Error::Rpc { code, .. }) => assert_eq!(code, -32601),
        other => panic!("{other:?}"),
    }
    assert!(log_lines(&f).iter().any(|l| l["unexpected"]["method"] == "thread/fork"));
}

#[tokio::test]
async fn request_times_out() {
    let f = fake(&[json!({"sleep_ms": 3000}), json!({"expect": "initialize", "result": {}})]).await;
    let (c, _rx) = connect(&f.url, "secret-token", Duration::from_millis(300), None)
        .await
        .unwrap();
    match c.request("initialize", json!({})).await {
        Err(Error::Timeout(m)) => assert_eq!(m, "initialize"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn server_request_and_reply() {
    let f = fake(&[
        json!({"expect": "turn/start", "result": {"turn": {"id": "$turn:1"}}}),
        json!({"send": {"id": "$req:1", "method": "item/commandExecution/requestApproval",
            "params": {"turnId": "$turn:1", "itemId": "$item:1"}}}),
        json!({"expect_reply": "$req:1"}),
        json!({"send": {"method": "serverRequest/resolved", "params": {"requestId": "$req:1"}}}),
    ])
    .await;
    let (c, mut rx) = connect(&f.url, "secret-token", Duration::from_secs(5), None)
        .await
        .unwrap();
    let r = c.request("turn/start", json!({})).await.unwrap();
    assert_eq!(r["turn"]["id"], "turn-1");
    let Incoming::Request { id, method, params } = rx.recv().await.unwrap() else {
        panic!("requête attendue");
    };
    assert_eq!(method, "item/commandExecution/requestApproval");
    assert_eq!(params["itemId"], "item-1");
    assert_eq!(id, json!(1));
    c.respond(&id, json!({"decision": "accept"})).unwrap();
    let Incoming::Notification { method, params } = rx.recv().await.unwrap() else {
        panic!("notification attendue");
    };
    assert_eq!(method, "serverRequest/resolved");
    assert_eq!(params["requestId"], 1);
    let log = log_lines(&f);
    let reply = log.iter().find_map(|l| l.get("reply")).expect("réponse notée");
    assert_eq!(reply["result"]["decision"], "accept");
}

#[tokio::test]
async fn closed_connection_fails_pending_requests_and_ends_the_stream() {
    let f = fake(&[json!({"sleep_ms": 200}), json!({"close": true})]).await;
    let (c, mut rx) = connect(&f.url, "secret-token", Duration::from_secs(5), None)
        .await
        .unwrap();
    match c.request("initialize", json!({})).await {
        Err(Error::Closed) => {}
        other => panic!("{other:?}"),
    }
    assert!(rx.recv().await.is_none());
}

#[tokio::test]
async fn bad_token_is_refused() {
    let f = fake(&[]).await;
    assert!(matches!(
        connect(&f.url, "wrong", Duration::from_secs(5), None).await,
        Err(Error::Connect(_))
    ));
}
