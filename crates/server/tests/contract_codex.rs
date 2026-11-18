//! Test de contrat : le vrai `codex exec` contre EngineX + faux llama-server
//! scripté (`tests/contract/script.json`). Ignoré par défaut ; lancé par
//! `scripts/contract-codex.sh`, qui installe `@openai/codex@latest` et pose
//! `ENGINEX_CODEX_BIN`.
//!
//! Tour 1 : raisonnement + appel de l'outil MCP `echo` (nom à plat
//! `mcp__test__echo`) ; tour 2 : `exec_command` avec un `apply_patch` ;
//! tour 3 : message final.

mod common;

use std::{path::PathBuf, time::Duration};

use common::{env_with, fake_bin, make_active_preset, new_key, serve_tcp};
use serde_json::Value;

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/contract/script.json")
}

#[tokio::test]
#[ignore = "lancé par scripts/contract-codex.sh (ENGINEX_CODEX_BIN)"]
async fn codex_exec_patches_a_file_through_enginex() {
    let Some(codex) = std::env::var_os("ENGINEX_CODEX_BIN").map(PathBuf::from) else {
        panic!("ENGINEX_CODEX_BIN absent : lancer scripts/contract-codex.sh");
    };
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("chat.log");
    let e = env_with(&format!(
        "FAKE_CHAT_SCRIPT={} FAKE_CHAT_LOG={}",
        script().display(),
        log.display()
    ))
    .await;
    make_active_preset(&e).await;
    let key = new_key(&e, "contrat").await;
    let base = serve_tcp(e.router.clone()).await;
    let temperature = e.get("/api/presets").await.body[0]["settings"]["sampling"]["temperature"].clone();

    // config.toml donné par EngineX (Host = vraie adresse), plus le serveur MCP.
    let cfg: Value = reqwest::Client::new()
        .get(format!("{base}/api/codex-config"))
        .header("cookie", &e.cookie)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cfg["base_url"], format!("{base}/v1"));
    let home = tmp.path().join("codex-home");
    std::fs::create_dir_all(&home).unwrap();
    let toml = format!(
        "{}\n[mcp_servers.test]\ncommand = {:?}\nargs = [\"mcp\"]\n",
        cfg["toml"].as_str().unwrap(),
        fake_bin().display().to_string()
    );
    std::fs::write(home.join("config.toml"), toml).unwrap();
    let work = tmp.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(work.join("hello.txt"), "hello\n").unwrap();

    let out = tokio::time::timeout(
        Duration::from_secs(180),
        tokio::process::Command::new(&codex)
            .args([
                "exec",
                "--skip-git-repo-check",
                "--json",
                "--dangerously-bypass-approvals-and-sandbox",
                "-C",
            ])
            .arg(&work)
            .arg("Remplace hello par bonjour dans hello.txt")
            .env("CODEX_HOME", &home)
            .env("ENGINEX_API_KEY", &key)
            .stdin(std::process::Stdio::null())
            .output(),
    )
    .await
    .expect("codex exec trop long")
    .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let sent: Vec<Value> = std::fs::read_to_string(&log)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let ctx = format!(
        "codex : {}\n--- stdout\n{stdout}\n--- stderr\n{stderr}\n--- requêtes reçues : {}",
        out.status,
        sent.len()
    );

    // Le patch est appliqué.
    let hello = std::fs::read_to_string(work.join("hello.txt")).unwrap();
    assert_eq!(hello.trim(), "bonjour", "{ctx}");
    assert!(sent.len() >= 3, "{ctx}");

    // Requête 2 : sortie de l'outil MCP rendue (bon namespace, outil exécuté) et
    // raisonnement du tour 1 renvoyé.
    let m2 = sent[1]["messages"].as_array().unwrap();
    let tool = m2
        .iter()
        .find(|m| m["role"] == "tool" && m["tool_call_id"] == "call_echo")
        .unwrap_or_else(|| panic!("message tool de echo absent : {}\n{ctx}", sent[1]));
    assert!(tool["content"].as_str().unwrap().contains("echo: hi"), "{tool}");
    let asst = m2
        .iter()
        .find(|m| m["role"] == "assistant")
        .expect("assistant du tour 1");
    assert_eq!(asst["reasoning_content"], "Je teste d'abord l'outil echo.", "{asst}");
    assert_eq!(asst["tool_calls"][0]["function"]["name"], "mcp__test__echo");

    // Échantillonnage du preset sur chaque requête.
    for (i, r) in sent.iter().enumerate() {
        assert_eq!(r["temperature"], temperature, "requête {i}");
    }

    // Déterminisme : les messages de la requête 2 commencent par ceux de la 1.
    let m1 = sent[0]["messages"].as_array().unwrap();
    assert!(m2.len() > m1.len());
    for (i, m) in m1.iter().enumerate() {
        assert_eq!(
            m.to_string(),
            m2[i].to_string(),
            "message {i} différent entre les tours"
        );
    }
}

// ---------------------------------------------------------------------------
// `codex app-server` piloté par le client d'EngineX seul (backend local) :
// faits V1 à V3 du plan 4a. Un scénario par fichier `model-<scénario>.json`.

use std::sync::Arc;

#[allow(unused_imports)]
use common::*;
use common::{Env, make_active_vision_preset};
use enginex_codex::{Client, Incoming, Recorder, record::shape_of, types};
use enginex_sandbox::{Mounts, Spec, local::Local};
use serde_json::json;
use tokio::sync::mpsc::UnboundedReceiver;

fn contract_file(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/contract")
        .join(name)
}

struct AsEnv {
    _e: Env,
    tmp: tempfile::TempDir,
    local: Local,
    spec: Spec,
    recorder: Arc<Recorder>,
    chat_log: PathBuf,
}

async fn as_env(scenario: &str, fake_env: &str, vision: bool) -> AsEnv {
    let Some(codex) = std::env::var_os("ENGINEX_CODEX_BIN").map(PathBuf::from) else {
        panic!("ENGINEX_CODEX_BIN absent : lancer scripts/contract-codex.sh");
    };
    let tmp = tempfile::tempdir().unwrap();
    let chat_log = tmp.path().join("chat.log");
    let e = env_with(&format!(
        "FAKE_CHAT_SCRIPT={} FAKE_CHAT_LOG={} {fake_env}",
        contract_file(&format!("model-{scenario}.json")).display(),
        chat_log.display()
    ))
    .await;
    let preset_id = if vision {
        make_active_vision_preset(&e).await
    } else {
        make_active_preset(&e).await
    };
    let key = new_key(&e, "app-server").await;
    let base = serve_tcp(e.router.clone()).await;
    let presets = e.get("/api/presets").await.body;
    let p = presets
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == preset_id)
        .unwrap()
        .clone();
    let root = tmp.path().join("session");
    let mounts = Mounts {
        topic: tmp.path().join("topic"),
        codex_home: root.join("codex"),
        run: root.join("run"),
        attachments: root.join("attachments"),
        cache: tmp.path().join("cache"),
    };
    for d in [
        &mounts.topic,
        &mounts.codex_home,
        &mounts.run,
        &mounts.attachments,
        &mounts.cache,
    ] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(mounts.run.join("token"), "contract-token").unwrap();
    std::fs::write(
        mounts.codex_home.join("config.toml"),
        enginex_server::codex_config::render(
            p["name"].as_str().unwrap(),
            p["settings"]["ctx"].as_u64().unwrap(),
            &format!("{base}/v1"),
        ),
    )
    .unwrap();
    let spec = Spec {
        session_id: format!("contract-{scenario}"),
        owner: "contract".into(),
        preset: format!("{preset_id}:0"),
        mounts,
        env: vec![("ENGINEX_API_KEY".into(), key)],
    };
    AsEnv {
        local: Local::new(codex, tmp.path().join("local")),
        _e: e,
        tmp,
        spec,
        recorder: Arc::new(Recorder::new(types::WHITELIST)),
        chat_log,
    }
}

impl AsEnv {
    async fn launch(&self) -> (Client, UnboundedReceiver<Incoming>) {
        let h = self.local.start(&self.spec).await.unwrap();
        let http = reqwest::Client::new();
        let mut ready = false;
        for _ in 0..300 {
            if http
                .get(&h.readyz_url)
                .send()
                .await
                .is_ok_and(|r| r.status().is_success())
            {
                ready = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(
            ready,
            "app-server pas prêt : {}",
            self.local.logs(&self.spec.session_id).await
        );
        let (c, rx) = enginex_codex::connect(
            &h.ws_url,
            "contract-token",
            Duration::from_secs(30),
            Some(self.recorder.clone()),
        )
        .await
        .unwrap();
        c.initialize("0.1.0").await.unwrap();
        (c, rx)
    }

    fn cwd(&self) -> String {
        self.spec.mounts.topic.display().to_string()
    }

    fn chat_requests(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.chat_log)
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}

/// Lit jusqu'à `turn/completed` ; `on_request` répond aux requêtes du serveur.
async fn until_turn_completed(
    c: &Client,
    rx: &mut UnboundedReceiver<Incoming>,
    mut on_request: impl FnMut(&str, &Value) -> Value,
    mut on_notif: impl FnMut(&str, &Value),
) -> (Value, Vec<String>) {
    let mut methods = Vec::new();
    loop {
        let m = tokio::time::timeout(Duration::from_secs(120), rx.recv())
            .await
            .expect("tour trop long")
            .expect("connexion fermée");
        match m {
            Incoming::Notification { method, params } => {
                on_notif(&method, &params);
                methods.push(method.clone());
                if method == "turn/completed" {
                    return (params["turn"].clone(), methods);
                }
            }
            Incoming::Request { id, method, params } => {
                methods.push(format!("request:{method}"));
                let r = on_request(&method, &params);
                c.respond(&id, r).unwrap();
            }
        }
    }
}

fn no_request(m: &str, _: &Value) -> Value {
    panic!("requête inattendue : {m}")
}

async fn start_thread(c: &Client, cwd: &str, policy: &str) -> String {
    let r = c
        .request("thread/start", types::thread_start(cwd, policy))
        .await
        .unwrap();
    types::thread_id(&r).unwrap().to_string()
}

async fn start_turn(c: &Client, thread: &str, input: Vec<Value>, policy: &str) -> String {
    let r = c
        .request("turn/start", types::turn_start(thread, input, policy))
        .await
        .unwrap();
    types::turn_id(&r).unwrap().to_string()
}

#[tokio::test]
#[ignore = "lancé par scripts/contract-codex.sh (ENGINEX_CODEX_BIN)"]
async fn appserver_text() {
    let a = as_env("text", "", false).await;
    let (c, mut rx) = a.launch().await;
    let th = start_thread(&c, &a.cwd(), "never").await;
    start_turn(&c, &th, vec![types::text_input("Dis bonjour")], "never").await;
    let mut text = String::new();
    let (turn, methods) = until_turn_completed(&c, &mut rx, no_request, |m, p| {
        if m == "item/agentMessage/delta" {
            text.push_str(p["delta"].as_str().unwrap_or(""));
        }
    })
    .await;
    assert_eq!(turn["status"], "completed", "{turn}");
    assert_eq!(text, "Bonjour, je suis prêt.", "{methods:?}");
}

#[tokio::test]
#[ignore = "lancé par scripts/contract-codex.sh (ENGINEX_CODEX_BIN)"]
async fn appserver_command() {
    let a = as_env("command", "", false).await;
    let (c, mut rx) = a.launch().await;
    let th = start_thread(&c, &a.cwd(), "never").await;
    start_turn(&c, &th, vec![types::text_input("Lance echo")], "never").await;
    let mut cmd = Value::Null;
    let (turn, methods) = until_turn_completed(&c, &mut rx, no_request, |m, p| {
        if m == "item/completed" && p["item"]["type"] == "commandExecution" {
            cmd = p["item"].clone();
        }
    })
    .await;
    assert_eq!(turn["status"], "completed", "{turn}");
    assert!(
        cmd["aggregatedOutput"].as_str().unwrap_or("").contains("contrat"),
        "{cmd}\n{methods:?}"
    );
}

#[tokio::test]
#[ignore = "lancé par scripts/contract-codex.sh (ENGINEX_CODEX_BIN)"]
async fn appserver_interrupt() {
    let a = as_env("interrupt", "FAKE_CHUNK_DELAY_MS=150", false).await;
    let (c, mut rx) = a.launch().await;
    let th = start_thread(&c, &a.cwd(), "never").await;
    let turn_id = start_turn(&c, &th, vec![types::text_input("Parle longtemps")], "never").await;
    // Premier delta, puis interruption.
    loop {
        match tokio::time::timeout(Duration::from_secs(60), rx.recv())
            .await
            .unwrap()
            .unwrap()
        {
            Incoming::Notification { method, .. } if method == "item/agentMessage/delta" => break,
            _ => {}
        }
    }
    c.request("turn/interrupt", json!({"threadId": th, "turnId": turn_id}))
        .await
        .unwrap();
    let (turn, _) = until_turn_completed(&c, &mut rx, no_request, |_, _| {}).await;
    assert_eq!(turn["status"], "interrupted", "{turn}");
}

/// V1 : `untrusted` + `danger-full-access` → approbation demandée.
#[tokio::test]
#[ignore = "lancé par scripts/contract-codex.sh (ENGINEX_CODEX_BIN)"]
async fn appserver_approval() {
    let a = as_env("approval", "", false).await;
    let (c, mut rx) = a.launch().await;
    let th = start_thread(&c, &a.cwd(), "untrusted").await;
    start_turn(&c, &th, vec![types::text_input("Crée made.txt")], "untrusted").await;
    let mut asked = Vec::new();
    let (turn, methods) = until_turn_completed(
        &c,
        &mut rx,
        |m, p| {
            asked.push((m.to_string(), p.clone()));
            json!({"decision": "accept"})
        },
        |_, _| {},
    )
    .await;
    assert_eq!(turn["status"], "completed", "{turn}");
    assert!(
        asked.iter().any(|(m, _)| m == "item/commandExecution/requestApproval"),
        "V1 faux : aucune approbation demandée ({methods:?})"
    );
    assert!(a.spec.mounts.topic.join("made.txt").exists(), "{methods:?}");
}

/// V2 : `localImage` → `image_url` en data: dans la requête chat.
#[tokio::test]
#[ignore = "lancé par scripts/contract-codex.sh (ENGINEX_CODEX_BIN)"]
async fn appserver_image() {
    let a = as_env("image", "", true).await;
    let png = a.spec.mounts.attachments.join("carre.png");
    image::RgbImage::from_pixel(16, 16, image::Rgb([200, 20, 20]))
        .save(&png)
        .unwrap();
    let (c, mut rx) = a.launch().await;
    let th = start_thread(&c, &a.cwd(), "never").await;
    start_turn(
        &c,
        &th,
        vec![
            types::text_input("Décris l'image"),
            types::local_image(&png.display().to_string()),
        ],
        "never",
    )
    .await;
    let (turn, _) = until_turn_completed(&c, &mut rx, no_request, |_, _| {}).await;
    assert_eq!(turn["status"], "completed", "{turn}");
    let sent = a.chat_requests();
    let raw = serde_json::to_string(&sent).unwrap();
    assert!(
        raw.contains("\"image_url\"") && raw.contains("data:image/png;base64,"),
        "V2 faux : pas d'image_url data: dans {} requêtes",
        sent.len()
    );
}

/// V3 : app-server relancé sur le même CODEX_HOME, `thread/resume` reprend le
/// fil et `thread/turns/list` rend le tour précédent.
#[tokio::test]
#[ignore = "lancé par scripts/contract-codex.sh (ENGINEX_CODEX_BIN)"]
async fn appserver_resume() {
    let a = as_env("resume", "", false).await;
    let (c, mut rx) = a.launch().await;
    let th = start_thread(&c, &a.cwd(), "never").await;
    let t1 = start_turn(&c, &th, vec![types::text_input("Premier")], "never").await;
    let (turn, _) = until_turn_completed(&c, &mut rx, no_request, |_, _| {}).await;
    assert_eq!(turn["status"], "completed");
    c.close();
    drop(rx);
    a.local.stop(&a.spec.session_id).await.unwrap();
    a.recorder.launch();

    let (c, mut rx) = a.launch().await;
    let r = c
        .request("thread/resume", types::thread_resume(&th, &a.cwd(), "never"))
        .await
        .expect("V3 faux : thread/resume refusé");
    assert_eq!(types::thread_id(&r), Some(th.as_str()));
    let list = c.request("thread/turns/list", types::turns_list(&th)).await.unwrap();
    let turns = list["data"].as_array().unwrap();
    let first = turns
        .iter()
        .find(|t| t["id"] == t1)
        .unwrap_or_else(|| panic!("tour 1 absent : {list}"));
    assert!(
        first["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["type"] == "agentMessage"),
        "{first}"
    );
    start_turn(&c, &th, vec![types::text_input("Deuxième")], "never").await;
    let (turn, _) = until_turn_completed(&c, &mut rx, no_request, |_, _| {}).await;
    assert_eq!(turn["status"], "completed");
    // Le modèle a reçu l'historique du premier tour.
    let sent = a.chat_requests();
    let last = sent.last().unwrap().to_string();
    assert!(last.contains("Premier tour terminé."), "historique perdu : {last}");
    let _ = &a.tmp;
}

// ---------------------------------------------------------------------------
// Même scénarios à travers le pont d'EngineX (routes `/api/sessions`,
// backend local, vrai Codex). Le client du pont enregistre ses échanges
// (`ENGINEX_RECORD_DIR`) : `UPDATE_FIXTURES=1` réécrit
// `appserver-<scénario>.jsonl` (relire le diff), sinon la suite
// `(type d'étape, méthode)` doit être celle du fichier.

struct Bridge {
    e: Env,
    record: PathBuf,
    chat_log: PathBuf,
    _tmp: tempfile::TempDir,
}

async fn bridge_env(scenario: &str, fake_env: &str, vision: bool) -> Bridge {
    let Some(codex) = std::env::var_os("ENGINEX_CODEX_BIN").map(PathBuf::from) else {
        panic!("ENGINEX_CODEX_BIN absent : lancer scripts/contract-codex.sh");
    };
    let tmp = tempfile::tempdir().unwrap();
    let chat_log = tmp.path().join("chat.log");
    let record = tmp.path().join("record");
    let port = common::free_port();
    let rec = record.clone();
    let e = common::env_cfg(
        &format!(
            "FAKE_CHAT_SCRIPT={} FAKE_CHAT_LOG={} {fake_env}",
            contract_file(&format!("model-{scenario}.json")).display(),
            chat_log.display()
        ),
        move |c| {
            c.sandbox.backend = enginex_server::config::SandboxBackend::Local;
            c.sandbox.codex_bin = codex;
            c.sandbox.api_url = format!("http://127.0.0.1:{port}/v1");
            c.sandbox.record_dir = Some(rec);
            c.timings.sandbox.ready_wait = Duration::from_secs(30);
            c.timings.sandbox.rpc_timeout = Duration::from_secs(30);
        },
    )
    .await;
    if vision {
        make_active_vision_preset(&e).await;
    } else {
        make_active_preset(&e).await;
    }
    serve_on(e.router.clone(), port).await;
    Bridge {
        e,
        record,
        chat_log,
        _tmp: tmp,
    }
}

impl Bridge {
    fn check_fixture(&self, scenario: &str, sid: &str) {
        let got_path = self.record.join(format!("{sid}.jsonl"));
        let got_raw = std::fs::read_to_string(&got_path).expect("enregistrement du pont absent");
        let path = contract_file(&format!("appserver-{scenario}.jsonl"));
        if std::env::var("UPDATE_FIXTURES").as_deref() == Ok("1") {
            std::fs::write(&path, &got_raw).unwrap();
            return;
        }
        let parse = |s: &str| -> Vec<Value> {
            s.lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| serde_json::from_str(l).unwrap())
                .collect()
        };
        let want = shape_of(&parse(
            &std::fs::read_to_string(&path).unwrap_or_else(|_| panic!("{} absent : UPDATE_FIXTURES=1", path.display())),
        ));
        let got = shape_of(&parse(&got_raw));
        // Les notifications de Codex (« send ») arrivent dans un ordre et en nombre
        // variables (deltas découpés autrement, usage avant ou après une requête) :
        // seul leur ensemble compte, deltas exclus. Le reste suit l'ordre exact.
        let split = |v: &[(String, String)]| {
            let steps: Vec<(String, String)> = v.iter().filter(|(k, _)| k != "send").cloned().collect();
            let notes: std::collections::BTreeSet<String> = v
                .iter()
                .filter(|(k, m)| k == "send" && !m.ends_with("Delta"))
                .map(|(_, m)| m.clone())
                .collect();
            (steps, notes)
        };
        let (want_steps, want_notes) = split(&want);
        let (got_steps, got_notes) = split(&got);
        for (i, (w, g)) in want_steps.iter().zip(got_steps.iter()).enumerate() {
            assert_eq!(w, g, "scénario {scenario}, échange {i} : attendu {w:?}, reçu {g:?}");
        }
        assert_eq!(
            want_steps.len(),
            got_steps.len(),
            "scénario {scenario} : nombre d'échanges"
        );
        assert_eq!(want_notes, got_notes, "scénario {scenario} : notifications");
    }

    async fn items(&self, sid: &str) -> Vec<Value> {
        self.e.get(&format!("/api/sessions/{sid}")).await.body["items"]
            .as_array()
            .unwrap()
            .clone()
    }
}

#[tokio::test]
#[ignore = "lancé par scripts/contract-codex.sh (ENGINEX_CODEX_BIN)"]
async fn bridge_text() {
    let b = bridge_env("text", "", false).await;
    let sid = new_session(&b.e, "auto").await;
    let mut ev = sse(&b.e, &format!("/api/sessions/{sid}/events")).await;
    ev.until("snapshot", |_| true).await;
    let r = turn(&b.e, &sid, "Dis bonjour").await;
    assert_eq!(r.status, 202, "{}", r.text);
    let t = ev.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["status"], "completed", "{t}\n{:?}", ev.names());
    let items = b.items(&sid).await;
    let kinds: Vec<&str> = items.iter().map(|i| i["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds.iter().filter(|k| **k == "userMessage").count(), 1, "{kinds:?}");
    assert!(
        items.iter().any(|i| i["data"]["text"] == "Bonjour, je suis prêt."),
        "{items:?}"
    );
    b.check_fixture("text", &sid);
}

#[tokio::test]
#[ignore = "lancé par scripts/contract-codex.sh (ENGINEX_CODEX_BIN)"]
async fn bridge_command() {
    let b = bridge_env("command", "", false).await;
    let sid = new_session(&b.e, "auto").await;
    let mut ev = sse(&b.e, &format!("/api/sessions/{sid}/events")).await;
    turn(&b.e, &sid, "Lance echo").await;
    let t = ev.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["status"], "completed", "{t}");
    let items = b.items(&sid).await;
    assert!(
        items.iter().any(|i| i["kind"] == "commandExecution"
            && i["data"]["aggregatedOutput"].as_str().unwrap_or("").contains("contrat")),
        "{items:?}"
    );
    b.check_fixture("command", &sid);
}

#[tokio::test]
#[ignore = "lancé par scripts/contract-codex.sh (ENGINEX_CODEX_BIN)"]
async fn bridge_approval() {
    let b = bridge_env("approval", "", false).await;
    let sid = new_session(&b.e, "prudent").await;
    let mut ev = sse(&b.e, &format!("/api/sessions/{sid}/events")).await;
    turn(&b.e, &sid, "Crée made.txt").await;
    let req = ev.until("request", |_| true).await;
    assert_eq!(req["method"], "item/commandExecution/requestApproval");
    let rid = req["rid"].as_str().unwrap();
    let r =
        b.e.send(
            "POST",
            &format!("/api/sessions/{sid}/requests/{rid}"),
            json!({"decision": "accept"}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    ev.until("request_resolved", |v| v["rid"] == rid).await;
    let t = ev.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["status"], "completed", "{t}");
    let topic =
        b.e.dir
            .path()
            .join("data/workspace/projects/general/topics/inbox/made.txt");
    assert!(topic.exists());
    b.check_fixture("approval", &sid);
}

#[tokio::test]
#[ignore = "lancé par scripts/contract-codex.sh (ENGINEX_CODEX_BIN)"]
async fn bridge_interrupt() {
    let b = bridge_env("interrupt", "FAKE_CHUNK_DELAY_MS=150", false).await;
    let sid = new_session(&b.e, "auto").await;
    let mut ev = sse(&b.e, &format!("/api/sessions/{sid}/events")).await;
    turn(&b.e, &sid, "Parle longtemps").await;
    ev.until("codex", |v| v["method"] == "item/agentMessage/delta").await;
    let r =
        b.e.send("POST", &format!("/api/sessions/{sid}/interrupt"), json!({}))
            .await;
    assert_eq!(r.status, 200, "{}", r.text);
    let t = ev.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["status"], "interrupted", "{t}");
    b.check_fixture("interrupt", &sid);
}

#[tokio::test]
#[ignore = "lancé par scripts/contract-codex.sh (ENGINEX_CODEX_BIN)"]
async fn bridge_image() {
    let b = bridge_env("image", "", true).await;
    let sid = new_session(&b.e, "auto").await;
    let up = upload(
        &b.e,
        &format!("/api/sessions/{sid}/attachments"),
        "image/png",
        png(16, 16),
    )
    .await;
    assert_eq!(up.status, 200, "{}", up.text);
    let mut ev = sse(&b.e, &format!("/api/sessions/{sid}/events")).await;
    let r =
        b.e.send(
            "POST",
            &format!("/api/sessions/{sid}/turns"),
            json!({"text": "Décris l'image", "attachments": [up.body["id"]]}),
        )
        .await;
    assert_eq!(r.status, 202, "{}", r.text);
    let t = ev.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["status"], "completed", "{t}");
    let chat = std::fs::read_to_string(&b.chat_log).unwrap();
    assert!(chat.contains("data:image/"), "image absente des requêtes chat");
    b.check_fixture("image", &sid);
}

#[tokio::test]
#[ignore = "lancé par scripts/contract-codex.sh (ENGINEX_CODEX_BIN)"]
async fn bridge_resume() {
    let b = bridge_env("resume", "", false).await;
    let sid = new_session(&b.e, "auto").await;
    let mut ev = sse(&b.e, &format!("/api/sessions/{sid}/events")).await;
    turn(&b.e, &sid, "Premier").await;
    ev.until("turn_stored", |_| true).await;
    // Sandbox arrêtée (archivage), puis reprise.
    for a in [true, false] {
        let r =
            b.e.send("PATCH", &format!("/api/sessions/{sid}"), json!({"archived": a}))
                .await;
        assert_eq!(r.status, 200);
    }
    ev.until("sandbox", |v| v["state"] == "stopped").await;
    turn(&b.e, &sid, "Deuxième").await;
    let t = ev.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["status"], "completed", "{t}");
    let items = b.items(&sid).await;
    let ids: Vec<&str> = items.iter().map(|i| i["item_id"].as_str().unwrap()).collect();
    let mut dedup = ids.clone();
    dedup.sort();
    dedup.dedup();
    assert_eq!(ids.len(), dedup.len(), "doublons : {ids:?}");
    assert_eq!(
        items.iter().filter(|i| i["kind"] == "agentMessage").count(),
        2,
        "{items:?}"
    );
    let chat = std::fs::read_to_string(&b.chat_log).unwrap();
    assert!(chat.lines().last().unwrap().contains("Premier tour terminé."));
    b.check_fixture("resume", &sid);
}

/// Un secret du coffre atteint les commandes de Codex (variable d'environnement
/// de la sandbox) et reste masqué dans l'historique et pour le modèle.
#[tokio::test]
#[ignore = "lancé par scripts/contract-codex.sh (ENGINEX_CODEX_BIN)"]
async fn bridge_secret() {
    const SECRET: &str = "e2e-secret12";
    let b = bridge_env("e2e", "", false).await;
    let r =
        b.e.send(
            "PUT",
            "/api/vault",
            json!({"scope": "global", "name": "E2E_SECRET", "value": SECRET}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    let sid = new_session(&b.e, "auto").await;
    let mut ev = sse(&b.e, &format!("/api/sessions/{sid}/events")).await;
    turn(&b.e, &sid, "Écris ok.txt").await;
    let t = ev.until("turn_stored", |_| true).await;
    assert_eq!(t["turn"]["status"], "completed", "{t}");
    let hist = b.e.get(&format!("/api/sessions/{sid}")).await;
    assert!(!hist.text.contains(SECRET), "{}", hist.text);
    let items = b.items(&sid).await;
    let out = items
        .iter()
        .find(|i| i["kind"] == "commandExecution")
        .map(|i| i["data"]["aggregatedOutput"].as_str().unwrap_or("").to_string())
        .expect("commande stockée");
    assert!(
        out.contains("***"),
        "secret absent de la commande (filtré par Codex ?) : {out:?}"
    );
    let chat = std::fs::read_to_string(&b.chat_log).unwrap();
    assert!(!chat.contains(SECRET));
    assert!(chat.contains("***"));
}
