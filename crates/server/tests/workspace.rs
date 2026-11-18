//! Projets et topics : slugs, doublons, dossiers, suppression d'un vide.

mod common;

use axum::http::StatusCode;
use common::{env, new_session};
use enginex_server::workspace::slugify;
use serde_json::json;

#[test]
fn slugs() {
    assert_eq!(slugify("Mon Projet été").as_deref(), Some("mon-projet-ete"));
    assert_eq!(slugify("  --Café!!  ").as_deref(), Some("cafe"));
    assert_eq!(slugify("a/b\\c..d").as_deref(), Some("a-b-c-d"));
    assert_eq!(slugify("???"), None);
    assert_eq!(slugify(&"x".repeat(80)).unwrap().len(), 40);
}

#[tokio::test]
async fn projects_topics_and_folders() {
    let e = env().await;
    let ws = e.dir.path().join("data/workspace/projects");
    let p = e.get("/api/projects").await.body;
    assert_eq!(p["projects"][0]["slug"], "general");
    assert_eq!(p["projects"][0]["topics"][0]["slug"], "inbox");
    assert!(ws.join("general/topics/inbox").is_dir());

    let r = e.send("POST", "/api/projects", json!({"name": "Site Boutique"})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.body["slug"], "site-boutique");
    let pid = r.body["id"].as_i64().unwrap();
    let r = e.send("POST", "/api/projects", json!({"name": "site boutique"})).await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert_eq!(r.body["error"], "name_taken");
    let r = e.send("POST", "/api/projects", json!({"name": "!!!"})).await;
    assert_eq!(r.body["error"], "invalid_name");

    let r = e
        .send("POST", &format!("/api/projects/{pid}/topics"), json!({"name": "Front"}))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let tid = r.body["id"].as_i64().unwrap();
    let dir = ws.join("site-boutique/topics/front");
    assert!(dir.is_dir());
    let r = e
        .send("POST", &format!("/api/projects/{pid}/topics"), json!({"name": "front"}))
        .await;
    assert_eq!(r.body["error"], "name_taken");

    // Non vide : fichier dans le dossier.
    std::fs::write(dir.join("index.html"), "x").unwrap();
    let r = e.send("DELETE", &format!("/api/topics/{tid}"), json!({})).await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert_eq!(r.body["error"], "not_empty");
    let r = e.send("DELETE", &format!("/api/projects/{pid}"), json!({})).await;
    assert_eq!(r.body["error"], "not_empty");
    std::fs::remove_file(dir.join("index.html")).unwrap();
    let r = e.send("DELETE", &format!("/api/projects/{pid}"), json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert!(!ws.join("site-boutique").exists());

    // Non vide : une session.
    new_session(&e, "auto").await;
    let inbox = e.get("/api/projects").await.body["projects"][0]["topics"][0].clone();
    assert_eq!(inbox["sessions"], 1);
    let r = e
        .send("DELETE", &format!("/api/topics/{}", inbox["id"]), json!({}))
        .await;
    assert_eq!(r.body["error"], "not_empty");
}
