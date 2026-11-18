//! Bout en bout Docker : vraie image devbox (vrai Codex dedans), vrai
//! docker-socket-proxy, EngineX dans le processus du test, faux llama-server.
//! Ignoré par défaut ; lancé par la CI (job `sandbox-e2e`), qui prépare le
//! réseau `enginex-sandbox-test` et le proxy (`CONTAINERS=1 IMAGES=1 POST=1`,
//! port 127.0.0.1:23750) et lance ce test sous l'uid 1000 :
//!   ENGINEX_TEST_DOCKER=1 ENGINEX_DEVBOX_IMAGE=enginex-devbox:ci \
//!     cargo test -p enginex-server --test sandbox_docker -- --ignored

mod common;

use std::{os::unix::fs::MetadataExt, path::PathBuf, time::Duration};

use common::*;
use enginex_server::config::SandboxBackend;
use serde_json::{Value, json};

const PROXY: &str = "http://127.0.0.1:23750";
const SECRET: &str = "e2e-secret12";

async fn until_long(ev: &mut Sse, name: &str, secs: u64) -> Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(secs);
    loop {
        match tokio::time::timeout(Duration::from_secs(35), ev.until(name, |_| true)).await {
            Ok(v) => return v,
            Err(_) if std::time::Instant::now() < deadline => continue,
            Err(_) => panic!("{name} attendu ; vus : {:?}", ev.names()),
        }
    }
}

#[tokio::test]
#[ignore = "lancé par la CI (job sandbox-e2e, ENGINEX_TEST_DOCKER=1)"]
async fn session_in_a_real_docker_sandbox() {
    assert_eq!(
        std::env::var("ENGINEX_TEST_DOCKER").as_deref(),
        Ok("1"),
        "ENGINEX_TEST_DOCKER=1 attendu"
    );
    let image = std::env::var("ENGINEX_DEVBOX_IMAGE").unwrap_or_else(|_| "enginex-devbox:ci".into());
    let tmp = tempfile::tempdir().unwrap();
    let chat_log = tmp.path().join("chat.log");
    let port = free_port();
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/contract/model-e2e.json");
    let e = env_cfg(
        &format!(
            "FAKE_CHAT_SCRIPT={} FAKE_CHAT_LOG={}",
            script.display(),
            chat_log.display()
        ),
        |c| {
            c.sandbox.backend = SandboxBackend::Docker;
            c.sandbox.docker_host = "tcp://127.0.0.1:23750".into();
            c.sandbox.network = "enginex-sandbox-test".into();
            c.sandbox.host_data = Some(c.data_dir.clone());
            c.sandbox.devbox_image = image;
            c.sandbox.api_url = format!("http://host.docker.internal:{port}/v1");
            c.sandbox.host_gateway = true;
            c.timings.sandbox.ready_wait = Duration::from_secs(120);
            c.timings.sandbox.rpc_timeout = Duration::from_secs(60);
        },
    )
    .await;
    let status = e.get("/api/sandbox/status").await.body;
    assert_eq!(status["docker"]["reachable"], true, "{status}");
    assert_eq!(status["image"]["present"], true, "{status}");
    make_active_preset(&e).await;
    // EngineX joignable depuis la sandbox (passerelle de l'hôte).
    let l = tokio::net::TcpListener::bind(("0.0.0.0", port)).await.unwrap();
    let router = e.router.clone();
    tokio::spawn(async move {
        axum::serve(l, router.into_make_service_with_connect_info::<std::net::SocketAddr>())
            .await
            .unwrap()
    });
    let r = e
        .send(
            "PUT",
            "/api/vault",
            json!({"scope": "global", "name": "E2E_SECRET", "value": SECRET}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text);

    let sid = new_session(&e, "auto").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    let r = turn(&e, &sid, "Écris ok.txt").await;
    assert_eq!(r.status, 202, "{}", r.text);
    let t = until_long(&mut ev, "turn_stored", 300).await;
    assert_eq!(t["turn"]["status"], "completed", "{t}\n{:?}", ev.names());

    // Fichier écrit sur l'hôte par l'uid 1000.
    let ok = e.dir.path().join("data/workspace/projects/general/topics/inbox/ok.txt");
    let meta = std::fs::metadata(&ok).expect("ok.txt absent");
    assert_eq!(meta.uid(), 1000);
    // Secret visible dans la sandbox, masqué dans l'historique et pour le modèle.
    let hist = e.get(&format!("/api/sessions/{sid}")).await;
    assert!(!hist.text.contains(SECRET), "{}", hist.text);
    let cmd = hist.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["kind"] == "commandExecution")
        .expect("commande stockée")
        .clone();
    assert!(
        cmd["data"]["aggregatedOutput"].as_str().unwrap_or("").contains("***"),
        "{cmd}"
    );
    let chat = std::fs::read_to_string(&chat_log).unwrap();
    assert!(!chat.contains(SECRET));
    assert!(chat.contains("***"));

    // Le proxy refuse de démarrer une commande dans la sandbox.
    let http = reqwest::Client::new();
    let name = format!("enginex-sbx-{}", &sid[..8]);
    let c: Value = http
        .get(format!("{PROXY}/containers/{name}/json"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let id = c["Id"].as_str().unwrap().to_string();
    let created = http
        .post(format!("{PROXY}/containers/{id}/exec"))
        .json(&json!({"Cmd": ["true"]}))
        .send()
        .await
        .unwrap();
    if created.status().is_success() {
        let exec: Value = created.json().await.unwrap();
        let start = http
            .post(format!("{PROXY}/exec/{}/start", exec["Id"].as_str().unwrap()))
            .json(&json!({"Detach": true}))
            .send()
            .await
            .unwrap();
        assert_eq!(
            start.status(),
            403,
            "le proxy laisse démarrer un exec : prévenir Michael"
        );
    } else {
        assert_eq!(created.status(), 403);
    }

    // Suppression : conteneur et clé interne supprimés.
    let r = e.send("DELETE", &format!("/api/sessions/{sid}"), json!({})).await;
    assert_eq!(r.status, 200, "{}", r.text);
    let gone = http
        .get(format!("{PROXY}/containers/{name}/json"))
        .send()
        .await
        .unwrap();
    assert_eq!(gone.status(), 404);
    let keys: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM api_keys WHERE internal = 1")
        .fetch_one(e.state.db())
        .await
        .unwrap();
    assert_eq!(keys, 0);
}
