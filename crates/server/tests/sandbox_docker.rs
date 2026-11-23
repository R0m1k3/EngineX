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

/// EngineX avec le backend Docker, servi sur `0.0.0.0:port` (joignable depuis
/// la sandbox par la passerelle de l'hôte).
async fn docker_env(model: &str) -> (Env, u16, PathBuf, tempfile::TempDir) {
    docker_env_with(model, None).await
}

/// `host_data` : dossier de l'hôte donné à Docker à la place de `<data>`.
async fn docker_env_with(model: &str, host_data: Option<PathBuf>) -> (Env, u16, PathBuf, tempfile::TempDir) {
    assert_eq!(
        std::env::var("ENGINEX_TEST_DOCKER").as_deref(),
        Ok("1"),
        "ENGINEX_TEST_DOCKER=1 attendu"
    );
    let image = std::env::var("ENGINEX_DEVBOX_IMAGE").unwrap_or_else(|_| "enginex-devbox:ci".into());
    let tmp = tempfile::tempdir().unwrap();
    let chat_log = tmp.path().join("chat.log");
    let port = free_port();
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/contract/{model}.json"));
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
            c.sandbox.host_data = Some(host_data.unwrap_or_else(|| c.data_dir.clone()));
            c.sandbox.devbox_image = image;
            c.sandbox.api_url = format!("http://host.docker.internal:{port}/v1");
            c.sandbox.host_gateway = true;
            c.web.preview_bind = Some("127.0.0.1:0".parse().unwrap());
            c.timings.sandbox.ready_wait = Duration::from_secs(120);
            c.timings.sandbox.rpc_timeout = Duration::from_secs(60);
        },
    )
    .await;
    let status = e.get("/api/sandbox/status").await.body;
    assert_eq!(status["docker"]["reachable"], true, "{status}");
    assert_eq!(status["image"]["present"], true, "{status}");
    // Version de Codex lue sur l'étiquette posée par la CI.
    assert!(status["image"]["codex"].is_string(), "{status}");
    make_active_preset(&e).await;
    // EngineX joignable depuis la sandbox (passerelle de l'hôte).
    let l = tokio::net::TcpListener::bind(("0.0.0.0", port)).await.unwrap();
    let router = e.router.clone();
    tokio::spawn(async move {
        axum::serve(l, router.into_make_service_with_connect_info::<std::net::SocketAddr>())
            .await
            .unwrap()
    });
    (e, port, chat_log, tmp)
}

#[tokio::test]
#[ignore = "lancé par la CI (job sandbox-e2e, ENGINEX_TEST_DOCKER=1)"]
async fn session_in_a_real_docker_sandbox() {
    let (e, _port, chat_log, _tmp) = docker_env("model-e2e").await;
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
    let ok = work_dir(&e, &sid).await.join("ok.txt");
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

async fn term_read_until(ws: &mut Ws, out: &mut String, want: &str) {
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    while !out.contains(want) {
        match tokio::time::timeout_at(deadline, ws.next()).await {
            Ok(Some(Ok(Message::Binary(b)))) => out.push_str(&String::from_utf8_lossy(&b)),
            Ok(Some(Ok(_))) => {}
            r => panic!("{want:?} attendu ({r:?}) ; reçu : {out:?}"),
        }
    }
}

async fn term_write(ws: &mut Ws, s: &str) {
    use futures_util::SinkExt;
    ws.send(tokio_tungstenite::tungstenite::Message::Binary(
        s.as_bytes().to_vec().into(),
    ))
    .await
    .unwrap();
}

fn git(repo: &std::path::Path, args: &[&str]) -> String {
    let o = std::process::Command::new("git")
        .arg("--git-dir")
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "git {args:?} : {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// Étape 4b dans le conteneur : vérification, terminal PTY (V8), retour
/// arrière d'un fichier écrit par la sandbox, course lien symbolique.
#[tokio::test]
#[ignore = "lancé par la CI (job sandbox-e2e, ENGINEX_TEST_DOCKER=1)"]
async fn code_session_in_a_real_docker_sandbox() {
    let (e, port, _chat_log, _tmp) = docker_env("model-e2e-4b").await;
    let r = e
        .send(
            "PUT",
            "/api/vault",
            json!({"scope": "global", "name": "E2E_SECRET", "value": SECRET}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    let sid = new_session(&e, "auto").await;
    let topic = work_dir(&e, &sid).await;
    std::fs::create_dir_all(topic.join(".enginex")).unwrap();
    std::fs::write(
        topic.join(".enginex/verify.toml"),
        "[[step]]\nname = \"fichier\"\nrun = \"grep -qx un un.txt && test \\\"$(id -u)\\\" = 1000\"\ntimeout = 60\n",
    )
    .unwrap();
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;

    // Tour, instantané, vérification dans le conteneur.
    turn(&e, &sid, "Écris un.txt").await;
    let t = until_long(&mut ev, "turn_stored", 300).await;
    assert_eq!(t["turn"]["status"], "completed", "{t}\n{:?}", ev.names());
    let tid = t["turn"]["id"].as_str().unwrap().to_string();
    let ch = until_long(&mut ev, "changes", 120).await;
    assert_eq!(ch["changes"]["files"][0]["path"], "un.txt", "{ch}");
    let v = loop {
        let v = until_long(&mut ev, "verify", 300).await;
        if v["status"] != "running" {
            break v;
        }
    };
    assert_eq!(v["status"], "passed", "{v}");

    // Terminal PTY dans le conteneur (V8).
    let base = format!("http://127.0.0.1:{port}");
    let mut ws = ws_connect(&base, &e.cookie, &format!("/api/sessions/{sid}/terminal"), Some(&base))
        .await
        .expect("terminal ouvert");
    let mut out = String::new();
    term_write(
        &mut ws,
        "echo \"u=$(id -u) s=$E2E_SECRET T=$TERM c=$(tput cols) t=$(tty)\"\n",
    )
    .await;
    term_read_until(&mut ws, &mut out, "s=*** T=xterm-256color c=80 t=/dev/pts/").await;
    assert!(out.contains("u=1000"), "{out}");
    assert!(!out.contains(SECRET), "{out}");

    // Aperçu (6c) : une app sur 127.0.0.1 dans le conteneur, jointe par le
    // relais de la devbox (port 4501, jeton de la sandbox).
    term_write(
        &mut ws,
        "python3 -m http.server 5173 --bind 127.0.0.1 >/dev/null 2>&1 &\n",
    )
    .await;
    let st = e
        .until(&format!("/api/sessions/{sid}/preview"), |v| {
            v["ports"].as_array().is_some_and(|p| p.contains(&json!(5173)))
        })
        .await;
    assert!(!st["ports"].as_array().unwrap().contains(&json!(4500)), "{st}");
    let r = e
        .send("POST", &format!("/api/sessions/{sid}/preview"), json!({"port": 5173}))
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    let grant = r.body["url"].as_str().unwrap().split("g=").nth(1).unwrap().to_string();
    let pbase = serve_tcp(enginex_server::preview::router(e.state.clone())).await;
    let page = reqwest::Client::new()
        .get(format!("{pbase}/"))
        .header("cookie", format!("enginex_preview={grant}"))
        .send()
        .await
        .unwrap();
    assert_eq!(page.status(), 200);
    assert!(
        page.text().await.unwrap().contains("un.txt"),
        "listage du topic attendu"
    );

    // Retour arrière des fichiers puis « Rétablir » : propriétaire 1000 gardé.
    let r = e
        .send(
            "POST",
            &format!("/api/sessions/{sid}/turns/{tid}/revert"),
            json!({"conversation": false}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    assert!(!topic.join("un.txt").exists());
    let r = e
        .send("POST", &format!("/api/sessions/{sid}/restore-undo"), json!({}))
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(std::fs::metadata(topic.join("un.txt")).unwrap().uid(), 1000);

    // Course lien symbolique : la sandbox alterne `piege` pendant 20 instantanés.
    term_write(
        &mut ws,
        "while :; do rm -f piege; echo PIEGE > piege; rm -f piege; ln -s /etc/hostname piege; done &\n",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let row = enginex_server::sessions::row(&e.state, &sid).await.unwrap();
    let f = enginex_server::snapshots::folder(&e.state, &row).await.unwrap();
    let mut seen = std::collections::HashSet::new();
    for i in 0..20 {
        let s = enginex_server::snapshots::snapshot(&e.state, &f, &format!("course {i}"), Default::default())
            .await
            .unwrap();
        let entry = git(&f.snap, &["ls-tree", &s.sha, "piege"]);
        if entry.trim().is_empty() {
            seen.insert("absent");
            continue;
        }
        let mode = entry.split_whitespace().next().unwrap().to_string();
        let blob = git(&f.snap, &["cat-file", "-p", &format!("{}:piege", s.sha)]);
        match mode.as_str() {
            "100644" => {
                // Vide si saisi entre la création et l'écriture ; jamais la cible.
                assert!(
                    "PIEGE\n".starts_with(&blob),
                    "contenu suivi à travers un lien : {blob:?}"
                );
                seen.insert("fichier");
            }
            "120000" => {
                assert_eq!(blob, "/etc/hostname");
                seen.insert("lien");
            }
            m => panic!("mode {m}"),
        }
    }
    eprintln!("course : {seen:?}");
    term_write(&mut ws, "kill %1\nexit\n").await;
    drop(ws);

    // Suppression : aucun conteneur d'aide restant.
    let r = e.send("DELETE", &format!("/api/sessions/{sid}"), json!({})).await;
    assert_eq!(r.status, 200, "{}", r.text);
    let filters = r#"{"label":["enginex.helper=1"]}"#;
    let left: Value = reqwest::Client::new()
        .get(format!("{PROXY}/containers/json"))
        .query(&[("all", "1"), ("filters", filters)])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(left.as_array().unwrap().len(), 0, "{left}");
}

/// Copie séparée dans Docker : clone par le conteneur d'aide, tour dans la
/// copie, « Ramener dans le topic », aucun conteneur d'aide restant.
#[tokio::test]
#[ignore = "lancé par la CI (job sandbox-e2e, ENGINEX_TEST_DOCKER=1)"]
async fn copy_session_in_a_real_docker_sandbox() {
    let (e, _port, _chat_log, _tmp) = docker_env("model-e2e-4b").await;
    let topic = make_git_topic(&e, Some("https://example.com/depot.git"));
    let (sid, branch) = new_copy_session(&e).await;
    let copy = enginex_server::snapshots::copy_dir(&e.state, &sid);
    assert_eq!(git_in(&copy, &["branch", "--show-current"]), branch);
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    turn(&e, &sid, "Écris un.txt").await;
    let t = until_long(&mut ev, "turn_stored", 300).await;
    assert_eq!(t["turn"]["status"], "completed", "{t}\n{:?}", ev.names());
    assert!(copy.join("un.txt").exists());
    assert!(!topic.join("un.txt").exists(), "le tour a écrit dans le topic");
    assert_eq!(std::fs::metadata(copy.join("un.txt")).unwrap().uid(), 1000);
    let r = e
        .send("POST", &format!("/api/sessions/{sid}/bring-back"), json!({}))
        .await;
    assert_eq!(r.body["result"], "merged", "{}", r.text);
    assert_eq!(std::fs::read_to_string(topic.join("un.txt")).unwrap(), "un\n");
    e.send("DELETE", &format!("/api/sessions/{sid}"), json!({})).await;
    let filters = r#"{"label":["enginex.helper=1"]}"#;
    let left: Value = reqwest::Client::new()
        .get(format!("{PROXY}/containers/json"))
        .query(&[("all", "1"), ("filters", filters)])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(left.as_array().unwrap().len(), 0, "{left}");
}

#[tokio::test]
#[ignore = "lancé par la CI (job sandbox-e2e, ENGINEX_TEST_DOCKER=1)"]
async fn wrong_host_data_is_refused_before_any_sandbox() {
    // Dossier de l'hôte qui n'est pas `<data>` (ENGINEX_HOST_DATA faux) : EX
    // écrirait ailleurs, la sandbox ne démarre pas et la raison est donnée.
    let other = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(other.path().join("workspace")).unwrap();
    let (e, _port, _log, _tmp) = docker_env_with("model-e2e", Some(other.path().to_path_buf())).await;
    let sid = new_session(&e, "auto").await;
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    let r = turn(&e, &sid, "Écris ok.txt").await;
    assert_eq!(r.status, 202, "{}", r.text);
    let t = until_long(&mut ev, "turn_stored", 300).await;
    assert_ne!(t["turn"]["status"], "completed", "{t}");
    let status = e.get("/api/sandbox/status").await.body;
    let err = status["host_data"]["error"].as_str().unwrap_or("");
    assert!(err.contains("ENGINEX_HOST_DATA"), "{status}");
    assert!(!work_dir(&e, &sid).await.join("ok.txt").exists());
}
