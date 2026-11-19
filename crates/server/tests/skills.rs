//! Skills : livrées au démarrage, API, copie dans `CODEX_HOME` sans suivre de
//! lien, `config.toml` écrit sans suivre de lien.

mod common;

use axum::http::StatusCode;
use common::{env, env_sessions_cfg, is_send, make_active_preset, new_session, script_with, sse, step_pos, turn};
use serde_json::{Value, json};

const DEMO: &str = "---\nname: demo\ndescription: Une skill de test\n---\n# Demo\n";

#[tokio::test]
async fn shipped_skills_and_api() {
    let e = env().await;
    let root = e.dir.path().join("data/workspace/skills");
    assert!(root.join("memory/SKILL.md").is_file());
    assert!(root.join("memory-compaction/SKILL.md").is_file());
    let l = e.get("/api/skills").await.body;
    let names: Vec<&str> = l["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["memory", "memory-compaction"]);
    assert_eq!(l["skills"][0]["shipped"], true);
    assert_eq!(l["skills"][0]["enabled"], true);

    // Une skill livrée modifiée n'est pas écrasée au redémarrage.
    let mine = "---\nname: memory\ndescription: à ma façon\n---\nperso\n";
    let r = e.send("PUT", "/api/skills/memory", json!({"content": mine})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    enginex_server::skills::boot(&e.state).unwrap();
    assert_eq!(e.get("/api/skills/memory").await.body["content"], mine);

    // Création, modification, frontmatter invalide.
    let r = e.send("PUT", "/api/skills/demo", json!({"content": DEMO})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(e.get("/api/skills/demo").await.body["content"], DEMO);
    let r = e
        .send(
            "PUT",
            "/api/skills/demo",
            json!({"content": DEMO.replace("# Demo", "# Demo v2")}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK);
    let r = e
        .send(
            "PUT",
            "/api/skills/demo",
            json!({"content": DEMO.replace("name: demo", "name: autre")}),
        )
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.body["error"], "invalid_skill");
    let r = e
        .send("PUT", "/api/skills/demo", json!({"content": "# sans frontmatter"}))
        .await;
    assert_eq!(r.body["error"], "invalid_skill");
    let r = e.send("PUT", "/api/skills/Pas%20Bon", json!({"content": DEMO})).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let big = format!("{DEMO}{}", "x".repeat(70 * 1024));
    let r = e.send("PUT", "/api/skills/demo", json!({"content": big})).await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(e.get("/api/skills/absente").await.status, StatusCode::NOT_FOUND);

    // Désactivation.
    let r = e
        .send("PUT", "/api/skills/demo/enabled", json!({"enabled": false}))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let l = e.get("/api/skills").await.body;
    let demo = l["skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "demo")
        .unwrap();
    assert_eq!(demo["enabled"], false);
    let r = e
        .send("PUT", "/api/skills/absente/enabled", json!({"enabled": false}))
        .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn install_into_copies_without_following_links() {
    let e = env().await;
    let root = e.dir.path().join("data/workspace/skills");
    let r = e.send("PUT", "/api/skills/demo", json!({"content": DEMO})).await;
    assert_eq!(r.status, StatusCode::OK);
    std::fs::create_dir_all(root.join("demo/scripts")).unwrap();
    std::fs::write(root.join("demo/scripts/run.sh"), "echo ok").unwrap();
    let outside = e.dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), "garde-moi").unwrap();
    std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("demo/lien.txt")).unwrap();
    e.send(
        "PUT",
        "/api/skills/memory-compaction/enabled",
        json!({"enabled": false}),
    )
    .await;

    let home = e.dir.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    // `skills` est un lien vers un dossier extérieur : retiré, l'extérieur intact.
    std::os::unix::fs::symlink(&outside, home.join("skills")).unwrap();
    enginex_server::skills::install_into(&e.state, &home).await.unwrap();
    let s = home.join("skills");
    assert!(!std::fs::symlink_metadata(&s).unwrap().file_type().is_symlink());
    assert!(s.join("memory/SKILL.md").is_file());
    assert_eq!(std::fs::read_to_string(s.join("demo/SKILL.md")).unwrap(), DEMO);
    assert_eq!(
        std::fs::read_to_string(s.join("demo/scripts/run.sh")).unwrap(),
        "echo ok"
    );
    assert!(
        std::fs::symlink_metadata(s.join("demo/lien.txt")).is_err(),
        "lien sauté"
    );
    assert!(!s.join("memory-compaction").exists(), "désactivée");
    assert_eq!(
        std::fs::read_to_string(outside.join("secret.txt")).unwrap(),
        "garde-moi"
    );
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 1);
    // Deuxième passage : remplacé proprement.
    enginex_server::skills::install_into(&e.state, &home).await.unwrap();
    assert!(s.join("demo/SKILL.md").is_file());
}

#[tokio::test]
async fn sandbox_start_replaces_a_config_toml_link() {
    let steps = script_with("text", |_| {});
    let end = step_pos(&steps, 0, is_send("turn/completed"));
    let script: Vec<Value> = steps[..=end].to_vec();
    let (e, fake) = env_sessions_cfg(&script, "", |_| {}).await;
    make_active_preset(&e).await;
    let sid = new_session(&e, "auto").await;
    let home = e.dir.path().join(format!("data/sessions/{sid}/codex"));
    std::fs::create_dir_all(&home).unwrap();
    let outside = e.dir.path().join("victime.toml");
    std::fs::write(&outside, "intact").unwrap();
    std::os::unix::fs::symlink(&outside, home.join("config.toml")).unwrap();
    let mut ev = sse(&e, &format!("/api/sessions/{sid}/events")).await;
    turn(&e, &sid, "un").await;
    ev.until("turn_stored", |_| true).await;
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "intact");
    let cfg = home.join("config.toml");
    assert!(std::fs::symlink_metadata(&cfg).unwrap().is_file());
    let toml = std::fs::read_to_string(&cfg).unwrap();
    assert!(toml.contains("[mcp_servers.enginex]"), "{toml}");
    assert!(toml.contains("bearer_token_env_var = \"ENGINEX_API_KEY\""));
    assert!(toml.contains("default_tools_approval_mode = \"approve\""));
    assert!(toml.contains("/mcp\""), "{toml}");
    assert!(home.join("skills/memory/SKILL.md").is_file());
    assert!(fake.unexpected().is_empty(), "{:?}", fake.unexpected());
}
