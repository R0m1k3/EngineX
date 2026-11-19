//! Tableau des tâches : API de l'UI, outils MCP `tasks_*`, fichier
//! `workspace/tasks.json` (champs inconnus gardés, écritures concurrentes).

mod common;

use axum::http::StatusCode;
use common::{env, mcp_tool, new_session, sandbox_key};
use serde_json::{Value, json};

fn file(e: &common::Env) -> Value {
    serde_json::from_str(&std::fs::read_to_string(e.dir.path().join("data/workspace/tasks.json")).unwrap()).unwrap()
}

#[tokio::test]
async fn api_crud() {
    let e = env().await;
    let b = e.get("/api/tasks").await;
    assert_eq!(b.status, StatusCode::OK);
    assert_eq!(b.body, json!({"version": 1, "projects": [], "tasks": []}));

    let p = e.send("POST", "/api/tasks/projects", json!({"name": "Magasin"})).await;
    assert_eq!(p.status, StatusCode::OK, "{}", p.text);
    let pid = p.body["id"].as_str().unwrap().to_string();
    assert_eq!(
        e.send("POST", "/api/tasks/projects", json!({"name": "magasin"}))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );

    let t = e
        .send(
            "POST",
            "/api/tasks",
            json!({"title": " Commander les cartons ", "project_id": pid, "due": "2026-12-01"}),
        )
        .await;
    assert_eq!(t.status, StatusCode::OK, "{}", t.text);
    let id = t.body["id"].as_str().unwrap().to_string();
    assert_eq!(t.body["title"], "Commander les cartons");
    assert_eq!(t.body["status"], "todo");
    assert_eq!(t.body["created_by"], "michael");
    let t2 = e.send("POST", "/api/tasks", json!({"title": "Deux"})).await.body;
    assert!(t2["order"].as_f64() > t.body["order"].as_f64());

    for bad in [
        json!({"title": ""}),
        json!({"title": "a\nb"}),
        json!({"title": "x", "due": "01/12/2026"}),
        json!({"title": "x", "project_id": "inconnu"}),
        json!({"title": "x", "couleur": "rouge"}),
    ] {
        let r = e.send("POST", "/api/tasks", bad.clone()).await;
        assert_ne!(r.status, StatusCode::OK, "{bad}");
    }

    // Passage en cours puis fait : date de fin posée, puis retirée.
    let r = e
        .send("PATCH", &format!("/api/tasks/{id}"), json!({"status": "done"}))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert!(r.body["done_at"].is_i64());
    let r = e
        .send(
            "PATCH",
            &format!("/api/tasks/{id}"),
            json!({"status": "doing", "due": null, "notes": "chez Raja"}),
        )
        .await;
    assert!(r.body.get("done_at").is_none() && r.body["due"].is_null(), "{}", r.body);
    assert_eq!(r.body["notes"], "chez Raja");
    assert_eq!(
        e.send("PATCH", &format!("/api/tasks/{id}"), json!({"status": "fini"}))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        e.send("PATCH", "/api/tasks/absente", json!({"status": "done"}))
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    // Projet supprimé : la tâche reste, sans projet.
    let r = e.send("DELETE", &format!("/api/tasks/projects/{pid}"), json!({})).await;
    assert_eq!(r.status, StatusCode::OK);
    let b = e.get("/api/tasks").await.body;
    assert!(b["projects"].as_array().unwrap().is_empty());
    let t = b["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == id.as_str())
        .unwrap();
    assert!(t["project_id"].is_null());

    let r = e.send("DELETE", &format!("/api/tasks/{id}"), json!({})).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(e.get("/api/tasks").await.body["tasks"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn unknown_fields_are_kept() {
    let e = env().await;
    let path = e.dir.path().join("data/workspace/tasks.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        json!({"version": 1, "theme": "sombre",
               "projects": [{"id": "p1", "name": "Maison", "color": "#c00"}],
               "tasks": [{"id": "t1", "title": "Tondre", "status": "todo", "project_id": "p1",
                          "labels": ["jardin"], "order": 1.0}]})
        .to_string(),
    )
    .unwrap();
    let r = e.send("PATCH", "/api/tasks/t1", json!({"status": "done"})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let f = file(&e);
    assert_eq!(f["theme"], "sombre");
    assert_eq!(f["projects"][0]["color"], "#c00");
    assert_eq!(f["tasks"][0]["labels"], json!(["jardin"]));
    assert_eq!(f["tasks"][0]["status"], "done");
}

#[tokio::test]
async fn mcp_tools_and_concurrency() {
    let e = env().await;
    let sid = new_session(&e, "auto").await;
    let key = sandbox_key(&e, &sid).await;

    let (t, err) = mcp_tool(&e, &key, "tasks_list", json!({})).await;
    assert!(!err && t == "(aucune tâche)", "{t}");
    let (t, err) = mcp_tool(
        &e,
        &key,
        "tasks_add",
        json!({"title": "Relire le bail", "project": "Maison", "due": "2026-12-24", "notes": "clause 4\nvoir notaire"}),
    )
    .await;
    assert!(!err, "{t}");
    assert!(
        t.contains("(à faire) Relire le bail · projet Maison · échéance 2026-12-24 · notes : clause 4"),
        "{t}"
    );
    let id = t.split('[').nth(1).unwrap().split(']').next().unwrap().to_string();
    let f = file(&e);
    assert_eq!(f["projects"][0]["name"], "Maison");
    assert_eq!(f["tasks"][0]["created_by"], format!("session:{sid}"));

    let (t, err) = mcp_tool(&e, &key, "tasks_add", json!({"title": "x", "due": "demain"})).await;
    assert!(err && t.contains("AAAA-MM-JJ"), "{t}");
    let (t, err) = mcp_tool(&e, &key, "tasks_update", json!({"id": id})).await;
    assert!(err && t.contains("rien à modifier"), "{t}");
    let (t, err) = mcp_tool(&e, &key, "tasks_update", json!({"id": id, "status": "done", "due": ""})).await;
    assert!(!err && t.contains("(fait)") && !t.contains("échéance"), "{t}");
    let (t, _) = mcp_tool(&e, &key, "tasks_list", json!({})).await;
    assert_eq!(t, "(aucune tâche)");
    let (t, _) = mcp_tool(&e, &key, "tasks_list", json!({"status": "done", "project": "maison"})).await;
    assert!(t.contains("Relire le bail"), "{t}");
    let (t, err) = mcp_tool(&e, &key, "tasks_list", json!({"project": "Boulot"})).await;
    assert!(err && t.contains("introuvable"), "{t}");

    // 20 ajouts simultanés : aucun perdu.
    let (e2, key2) = (&e, &key);
    let h: Vec<_> = (0..20)
        .map(|i| async move { mcp_tool(e2, key2, "tasks_add", json!({"title": format!("tâche {i}")})).await })
        .collect();
    for (t, err) in futures_util::future::join_all(h).await {
        assert!(!err, "{t}");
    }
    assert_eq!(file(&e)["tasks"].as_array().unwrap().len(), 21);
}
