//! Copie séparée (backend local, vrai git) : création, « Ramener dans le
//! topic » (fusion, topic sale, conflit).

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;

#[tokio::test]
async fn copy_needs_a_git_topic() {
    let (e, _fake) = env_sessions(&[]).await;
    let r = e
        .send("POST", "/api/sessions", json!({"topic_id": 1, "copy": true}))
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.body["error"], "not_git");
    let p = e.get("/api/projects").await.body;
    assert_eq!(p["projects"][0]["topics"][0]["git"], false);
    make_git_topic(&e, None);
    let p = e.get("/api/projects").await.body;
    assert_eq!(p["projects"][0]["topics"][0]["git"], true);
}

#[tokio::test]
async fn copy_is_a_clone_on_its_own_branch() {
    let (e, _fake) = env_sessions(&[]).await;
    let topic = make_git_topic(&e, Some("https://example.com/depot.git"));
    std::fs::write(topic.join("pas-commite.txt"), "x").unwrap();
    let (sid, branch) = new_copy_session(&e).await;
    assert_eq!(branch, format!("enginex/inbox-{}", &sid.replace('-', "")[..8]));
    let copy = enginex_server::snapshots::copy_dir(&e.state, &sid);
    assert_eq!(git_in(&copy, &["branch", "--show-current"]), branch);
    assert_eq!(
        git_in(&copy, &["remote", "get-url", "origin"]),
        "https://example.com/depot.git"
    );
    assert_eq!(std::fs::read_to_string(copy.join("a.txt")).unwrap(), "un\n");
    assert!(!copy.join("pas-commite.txt").exists(), "le non commité n'est pas copié");
    // La session voit la copie (panneau Fichiers).
    let s = e.get(&format!("/api/sessions/{sid}")).await.body;
    assert_eq!(s["session"]["copy_branch"], branch.as_str());
    let t = e.get(&format!("/api/sessions/{sid}/tree")).await.body;
    let names: Vec<&str> = t["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"a.txt") && !names.contains(&"pas-commite.txt"),
        "{names:?}"
    );
    // Sans origin dans le topic : pas d'origin dans la copie.
    git_in(&topic, &["remote", "remove", "origin"]);
    let (sid2, _) = new_copy_session(&e).await;
    let copy2 = enginex_server::snapshots::copy_dir(&e.state, &sid2);
    assert_eq!(git_in(&copy2, &["remote"]), "");
    // Supprimer la session supprime la copie.
    e.send("DELETE", &format!("/api/sessions/{sid2}"), json!({})).await;
    assert!(!copy2.exists());
}

#[tokio::test]
async fn bring_back_merges_into_a_clean_topic() {
    let (e, _fake) = env_sessions(&[]).await;
    let topic = make_git_topic(&e, None);
    let (sid, branch) = new_copy_session(&e).await;
    let copy = enginex_server::snapshots::copy_dir(&e.state, &sid);
    std::fs::write(copy.join("nouveau.txt"), "de la copie\n").unwrap();
    e.send(
        "PATCH",
        &format!("/api/sessions/{sid}"),
        json!({"title": "Ajout du fichier"}),
    )
    .await;
    let r = e
        .send("POST", &format!("/api/sessions/{sid}/bring-back"), json!({}))
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(r.body["result"], "merged", "{}", r.text);
    assert_eq!(
        std::fs::read_to_string(topic.join("nouveau.txt")).unwrap(),
        "de la copie\n"
    );
    assert_eq!(
        git_in(&topic, &["log", "-1", "--format=%s"]),
        "EngineX : Ajout du fichier"
    );
    assert_eq!(
        git_in(&topic, &["log", "-1", "--format=%an <%ae>"]),
        "EngineX <enginex@localhost>"
    );
    assert!(!git_in(&topic, &["branch", "--list", &branch]).is_empty());
    assert!(copy.exists(), "la copie reste");
}

#[tokio::test]
async fn bring_back_keeps_only_the_branch_when_the_topic_is_dirty_or_conflicts() {
    let (e, _fake) = env_sessions(&[]).await;
    let topic = make_git_topic(&e, None);
    let (sid, branch) = new_copy_session(&e).await;
    let copy = enginex_server::snapshots::copy_dir(&e.state, &sid);
    std::fs::write(copy.join("a.txt"), "copie\n").unwrap();
    // Topic sale (fichier suivi modifié).
    std::fs::write(topic.join("a.txt"), "topic en cours\n").unwrap();
    let r = e
        .send("POST", &format!("/api/sessions/{sid}/bring-back"), json!({}))
        .await;
    assert_eq!(r.body["result"], "branch_only", "{}", r.text);
    assert_eq!(r.body["reason"], "topic_dirty");
    assert!(!git_in(&topic, &["branch", "--list", &branch]).is_empty());
    assert_eq!(
        std::fs::read_to_string(topic.join("a.txt")).unwrap(),
        "topic en cours\n"
    );
    // Conflit : le topic a commité autre chose sur a.txt.
    git_in(&topic, &["commit", "-q", "-am", "côté topic"]);
    let r = e
        .send("POST", &format!("/api/sessions/{sid}/bring-back"), json!({}))
        .await;
    assert_eq!(r.body["result"], "branch_only", "{}", r.text);
    assert_eq!(r.body["reason"], "conflict");
    assert!(!topic.join(".git/MERGE_HEAD").exists(), "fusion annulée");
    assert_eq!(git_in(&topic, &["status", "--porcelain", "--untracked-files=no"]), "");
}
