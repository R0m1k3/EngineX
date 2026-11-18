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
