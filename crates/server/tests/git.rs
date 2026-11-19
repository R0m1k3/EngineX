//! Page Git : lecture par gix (rien d'exécuté par la config du dépôt), push /
//! fetch / pull / fusion par le conteneur d'aide (backend local ici), amont
//! nu local comme `origin`.

mod common;

use axum::http::StatusCode;
use common::{env, git_in, make_git_topic};
use serde_json::{Value, json};

fn bare(e: &common::Env) -> std::path::PathBuf {
    let up = e.dir.path().join("up.git");
    std::fs::create_dir_all(&up).unwrap();
    git_in(&up, &["init", "-q", "--bare", "-b", "main"]);
    up
}

fn branch<'a>(v: &'a Value, name: &str) -> &'a Value {
    v["branches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap_or_else(|| panic!("branche {name} absente : {v}"))
}

#[tokio::test]
async fn push_fetch_pull_and_graph() {
    let e = env().await;
    let up = bare(&e);
    let topic = make_git_topic(&e, Some(&up.display().to_string()));

    let r = e.get("/api/git/repos").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.body["repos"][0]["topic_id"], 1);
    assert_eq!(r.body["repos"][0]["head"], "main");

    // Premier push : amont posé, origin/main connu.
    let p = e.send("POST", "/api/git/repos/1/push", json!({})).await;
    assert_eq!(p.status, StatusCode::OK, "{}", p.text);
    assert_eq!(p.body["branch"], "main");
    assert_eq!(
        git_in(&up, &["rev-parse", "main"]),
        git_in(&topic, &["rev-parse", "HEAD"])
    );
    let v = e.get("/api/git/repos/1").await.body;
    let main = branch(&v, "main");
    assert_eq!(main["upstream"], "origin/main", "{v}");
    assert_eq!((main["ahead"].as_u64(), main["behind"].as_u64()), (Some(0), Some(0)));

    // Un commit local : en avance de 1.
    std::fs::write(topic.join("b.txt"), "deux\n").unwrap();
    git_in(&topic, &["add", "-A"]);
    git_in(&topic, &["commit", "-q", "-m", "ajout b"]);
    let v = e.get("/api/git/repos/1").await.body;
    assert_eq!(branch(&v, "main")["ahead"], 1);
    assert_eq!(v["commits"][0]["subject"], "ajout b");
    assert_eq!(v["commits"][0]["refs"][0]["name"], "main");
    assert_eq!(v["commits"].as_array().unwrap().len(), 2);
    // Détail du commit : diff calculé par EngineX.
    let sha = v["commits"][0]["sha"].as_str().unwrap();
    let c = e.get(&format!("/api/git/repos/1/commits/{sha}")).await;
    assert_eq!(c.status, StatusCode::OK, "{}", c.text);
    assert_eq!(c.body["files"][0]["path"], "b.txt");
    assert_eq!(c.body["files"][0]["status"], "added");
    assert!(c.body["diff"].as_str().unwrap().contains("+deux"), "{}", c.body["diff"]);
    assert_eq!(
        e.send("POST", "/api/git/repos/1/push", json!({})).await.status,
        StatusCode::OK
    );

    // Quelqu'un d'autre pousse : fetch → en retard, pull → avance rapide.
    let other = e.dir.path().join("autre");
    git_in(e.dir.path(), &["clone", "-q", &up.display().to_string(), "autre"]);
    std::fs::write(other.join("c.txt"), "trois\n").unwrap();
    git_in(&other, &["add", "-A"]);
    git_in(&other, &["commit", "-q", "-m", "ajout c"]);
    git_in(&other, &["push", "-q", "origin", "main"]);
    git_in(&other, &["push", "-q", "origin", "main:feature"]);
    let f = e.send("POST", "/api/git/repos/1/fetch", json!({})).await;
    assert_eq!(f.status, StatusCode::OK, "{}", f.text);
    let v = e.get("/api/git/repos/1").await.body;
    assert_eq!(branch(&v, "main")["behind"], 1, "{v}");
    assert!(
        v["remotes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["name"] == "origin/feature")
    );
    let p = e.send("POST", "/api/git/repos/1/pull", json!({})).await;
    assert_eq!(p.status, StatusCode::OK, "{}", p.text);
    assert!(topic.join("c.txt").exists());
    // Branche supprimée en amont : retirée au fetch suivant.
    git_in(&other, &["push", "-q", "origin", ":feature"]);
    e.send("POST", "/api/git/repos/1/fetch", json!({})).await;
    let v = e.get("/api/git/repos/1").await.body;
    assert!(
        !v["remotes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["name"] == "origin/feature"),
        "{v}"
    );

    // Push refusé (pas en avance rapide) : push_rejected, jamais forcé.
    std::fs::write(other.join("d.txt"), "quatre\n").unwrap();
    git_in(&other, &["add", "-A"]);
    git_in(&other, &["commit", "-q", "-m", "ajout d"]);
    git_in(&other, &["push", "-q", "origin", "main"]);
    std::fs::write(topic.join("e.txt"), "cinq\n").unwrap();
    git_in(&topic, &["add", "-A"]);
    git_in(&topic, &["commit", "-q", "-m", "ajout e"]);
    let p = e.send("POST", "/api/git/repos/1/push", json!({})).await;
    assert_eq!(p.status, StatusCode::CONFLICT, "{}", p.text);
    assert_eq!(p.body["error"], "push_rejected");
    // Pull en divergence : pas d'avance rapide.
    let p = e.send("POST", "/api/git/repos/1/pull", json!({})).await;
    assert_eq!(p.body["error"], "not_fast_forward", "{}", p.text);
}

#[tokio::test]
async fn branches_merge_and_conflict() {
    let e = env().await;
    let topic = make_git_topic(&e, None);
    let r = e
        .send("POST", "/api/git/repos/1/branches", json!({"branch": "essai"}))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let r = e
        .send("POST", "/api/git/repos/1/checkout", json!({"branch": "essai"}))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(git_in(&topic, &["branch", "--show-current"]), "essai");
    std::fs::write(topic.join("a.txt"), "un\nessai\n").unwrap();
    git_in(&topic, &["commit", "-q", "-am", "essai"]);
    // Modification en cours : pas de changement de branche.
    std::fs::write(topic.join("a.txt"), "brouillon\n").unwrap();
    let r = e
        .send("POST", "/api/git/repos/1/checkout", json!({"branch": "main"}))
        .await;
    assert_eq!(r.body["error"], "dirty", "{}", r.text);
    git_in(&topic, &["checkout", "-q", "--", "a.txt"]);
    e.send("POST", "/api/git/repos/1/checkout", json!({"branch": "main"}))
        .await;
    let r = e
        .send("POST", "/api/git/repos/1/merge", json!({"branch": "essai"}))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let v = e.get("/api/git/repos/1").await.body;
    assert_eq!(v["commits"][0]["parents"].as_array().unwrap().len(), 2, "{v}");

    // Conflit : fusion abandonnée, dossier propre.
    git_in(&topic, &["checkout", "-q", "-b", "conflit", "HEAD~1"]);
    std::fs::write(topic.join("a.txt"), "autre chose\n").unwrap();
    git_in(&topic, &["commit", "-q", "-am", "conflit"]);
    git_in(&topic, &["checkout", "-q", "main"]);
    let r = e
        .send("POST", "/api/git/repos/1/merge", json!({"branch": "conflit"}))
        .await;
    assert_eq!(r.body["error"], "merge_conflict", "{}", r.text);
    assert_eq!(git_in(&topic, &["status", "--porcelain"]), "");
    // Noms refusés.
    for b in ["-x", "a..b", "x.lock", "a b"] {
        let r = e.send("POST", "/api/git/repos/1/branches", json!({"branch": b})).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{b}");
    }
}

#[tokio::test]
async fn reading_never_runs_repo_config() {
    let e = env().await;
    let topic = make_git_topic(&e, None);
    let mark = e.dir.path().join("piege");
    let cmd = format!("touch {}", mark.display());
    git_in(&topic, &["config", "core.fsmonitor", &cmd]);
    git_in(&topic, &["config", "diff.external", &cmd]);
    git_in(&topic, &["config", "filter.x.smudge", &cmd]);
    git_in(&topic, &["config", "filter.x.clean", &cmd]);
    std::fs::write(topic.join(".gitattributes"), "* filter=x diff=x\n").unwrap();
    git_in(&topic, &["config", "diff.x.textconv", &cmd]);
    let hooks = topic.join(".git/hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    for h in ["post-checkout", "post-merge", "pre-commit", "reference-transaction"] {
        let p = hooks.join(h);
        std::fs::write(&p, format!("#!/bin/sh\n{cmd}\n")).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let v = e.get("/api/git/repos/1").await;
    assert_eq!(v.status, StatusCode::OK, "{}", v.text);
    let sha = v.body["commits"][0]["sha"].as_str().unwrap().to_string();
    let c = e.get(&format!("/api/git/repos/1/commits/{sha}")).await;
    assert_eq!(c.status, StatusCode::OK);
    assert!(!mark.exists(), "lecture : la config du dépôt a lancé une commande");
    // Les actions tournent dans le conteneur d'aide (backend local ici) : les
    // hooks y restent coupés.
    std::fs::remove_file(topic.join(".gitattributes")).unwrap();
    e.send("POST", "/api/git/repos/1/branches", json!({"branch": "b"}))
        .await;
    e.send("POST", "/api/git/repos/1/checkout", json!({"branch": "b"}))
        .await;
    assert!(!mark.exists(), "action : un hook a tourné");
    // `.git` en lien symbolique : pas un dépôt pour la page Git.
    let real = e.dir.path().join("vrai");
    std::fs::rename(topic.join(".git"), &real).unwrap();
    std::os::unix::fs::symlink(&real, topic.join(".git")).unwrap();
    let r = e.get("/api/git/repos").await;
    assert_eq!(r.body["repos"].as_array().unwrap().len(), 0, "{}", r.text);
}

#[tokio::test]
async fn tokens_only_go_to_their_host() {
    let e = env().await;
    for (name, value) in [
        ("GITHUB_TOKEN", "ghp_secret_github"),
        ("GITEA_TOKEN", "gitea_secret"),
        ("GITEA_URL", "https://git.example.org"),
    ] {
        let r = e
            .send(
                "PUT",
                "/api/vault",
                json!({"scope": "global", "name": name, "value": value}),
            )
            .await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    }
    let st = &e.state;
    let r = enginex_server::forge::resolve(st, 1, "git@github.com:o/r.git", false)
        .await
        .unwrap();
    assert_eq!(r.push_url, "https://github.com/o/r.git");
    assert_eq!(r.token.as_deref(), Some("ghp_secret_github"));
    let r = enginex_server::forge::resolve(st, 1, "https://git.example.org/Michael/site.git", false)
        .await
        .unwrap();
    assert_eq!(r.token.as_deref(), Some("gitea_secret"));
    assert_eq!(r.api.as_deref(), Some("https://git.example.org/api/v1"));
    assert_eq!((r.owner.as_str(), r.name.as_str()), ("Michael", "site"));
    // Autre hôte, ou http vers l'hôte Gitea : aucun jeton.
    for url in [
        "https://evil.example.com/o/r.git",
        "http://git.example.org/Michael/site.git",
        "https://github.com.evil.com/o/r.git",
        "https://user:pw@evil.example.com/o/r.git",
    ] {
        let r = enginex_server::forge::resolve(st, 1, url, false).await.unwrap();
        assert!(r.token.is_none(), "{url}");
        assert!(!r.push_url.contains("pw"), "{url}");
    }
    // Chemin local refusé hors backend local.
    assert!(
        enginex_server::forge::resolve(st, 1, "/tmp/x.git", false)
            .await
            .is_err()
    );
    assert!(
        enginex_server::forge::resolve(st, 1, "ext::sh -c id", false)
            .await
            .is_err()
    );
}
