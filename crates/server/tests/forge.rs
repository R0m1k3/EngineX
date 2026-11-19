//! Pull requests : faux GitHub et faux Gitea (liste, détail + diff +
//! commentaires, commentaire, fusion, création), jeton selon l'hôte ; commit /
//! push / pull request depuis une session.

mod common;

use std::sync::{Arc, Mutex};

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, put},
};
use common::{env_cfg, git_in, make_git_topic, new_session, serve_tcp};
use serde_json::{Value, json};

#[derive(Clone, Default)]
struct Seen(Arc<Mutex<Vec<(String, String, Value)>>>);

fn auth(h: &HeaderMap) -> String {
    h.get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

fn pr_json(n: u64, head: &str) -> Value {
    json!({
        "number": n, "title": "Ajout b", "state": "open", "draft": false,
        "user": {"login": "michael"}, "created_at": "2026-11-19T10:00:00Z",
        "updated_at": "2026-11-19T10:00:00Z", "head": {"ref": head}, "base": {"ref": "main"},
        "body": "corps", "html_url": format!("https://forge/pull/{n}"), "mergeable": true, "merged": false,
    })
}

/// Faux GitHub (`/repos/o/r/...`) et faux Gitea (`/api/v1/repos/o/r/...`).
async fn fake_forge(seen: Seen) -> String {
    async fn list(State(s): State<Seen>, h: HeaderMap) -> Json<Value> {
        s.0.lock().unwrap().push(("list".into(), auth(&h), Value::Null));
        Json(json!([pr_json(7, "feature")]))
    }
    async fn one(State(s): State<Seen>, h: HeaderMap, Path(n): Path<String>) -> axum::response::Response {
        s.0.lock().unwrap().push(("get".into(), auth(&h), json!(n)));
        let accept = h.get("accept").and_then(|v| v.to_str().ok()).unwrap_or("");
        if n.ends_with(".diff") || accept.contains("diff") {
            return "diff --git a/b.txt b/b.txt\n+deux\n".into_response();
        }
        Json(pr_json(n.parse().unwrap_or(0), "feature")).into_response()
    }
    async fn comments(State(s): State<Seen>, h: HeaderMap) -> Json<Value> {
        s.0.lock().unwrap().push(("comments".into(), auth(&h), Value::Null));
        Json(json!([{"user": {"login": "relecteur"}, "body": "ok pour moi", "created_at": "2026", "html_url": "u"}]))
    }
    async fn comment(State(s): State<Seen>, h: HeaderMap, Json(b): Json<Value>) -> Json<Value> {
        s.0.lock().unwrap().push(("comment".into(), auth(&h), b.clone()));
        Json(json!({"user": {"login": "michael"}, "body": b["body"], "created_at": "2026", "html_url": "u"}))
    }
    async fn merge(State(s): State<Seen>, h: HeaderMap, Json(b): Json<Value>) -> impl IntoResponse {
        s.0.lock().unwrap().push(("merge".into(), auth(&h), b));
        (StatusCode::OK, Json(json!({"merged": true})))
    }
    async fn create(State(s): State<Seen>, h: HeaderMap, Json(b): Json<Value>) -> Json<Value> {
        s.0.lock().unwrap().push(("create".into(), auth(&h), b.clone()));
        let mut p = pr_json(8, b["head"].as_str().unwrap_or(""));
        p["base"]["ref"] = b["base"].clone();
        Json(p)
    }
    async fn repo(State(s): State<Seen>, h: HeaderMap) -> Json<Value> {
        s.0.lock().unwrap().push(("repo".into(), auth(&h), Value::Null));
        Json(json!({"default_branch": "main"}))
    }
    let routes = |r: Router<Seen>, base: &str| {
        r.route(&format!("{base}/repos/o/r"), get(repo))
            .route(&format!("{base}/repos/o/r/pulls"), get(list).post(create))
            .route(&format!("{base}/repos/o/r/pulls/{{n}}"), get(one))
            .route(&format!("{base}/repos/o/r/pulls/{{n}}/merge"), put(merge).post(merge))
            .route(
                &format!("{base}/repos/o/r/issues/{{n}}/comments"),
                get(comments).post(comment),
            )
    };
    let app = routes(routes(Router::new(), ""), "/api/v1").with_state(seen);
    serve_tcp(app).await
}

async fn put_secret(e: &common::Env, name: &str, value: &str) {
    let r = e
        .send(
            "PUT",
            "/api/vault",
            json!({"scope": "global", "name": name, "value": value}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
}

#[tokio::test]
async fn github_pull_requests() {
    let seen = Seen::default();
    let api = fake_forge(seen.clone()).await;
    let e = env_cfg("", |c| c.web.github_api = api.clone()).await;
    make_git_topic(&e, Some("git@github.com:o/r.git"));
    put_secret(&e, "GITHUB_TOKEN", "ghp_jeton").await;

    let v = e.get("/api/git/repos/1").await;
    assert_eq!(v.body["forge"], "github", "{}", v.text);
    let l = e.get("/api/git/repos/1/pulls").await;
    assert_eq!(l.status, StatusCode::OK, "{}", l.text);
    assert_eq!(l.body["pulls"][0]["number"], 7);
    assert_eq!(l.body["pulls"][0]["author"], "michael");
    assert_eq!(l.body["pulls"][0]["head"], "feature");
    let d = e.get("/api/git/repos/1/pulls/7").await;
    assert_eq!(d.status, StatusCode::OK, "{}", d.text);
    assert!(d.body["diff"].as_str().unwrap().contains("+deux"));
    assert_eq!(d.body["comments"][0]["body"], "ok pour moi");
    let c = e
        .send("POST", "/api/git/repos/1/pulls/7/comments", json!({"body": "merci"}))
        .await;
    assert_eq!(c.status, StatusCode::OK, "{}", c.text);
    let m = e
        .send("POST", "/api/git/repos/1/pulls/7/merge", json!({"method": "squash"}))
        .await;
    assert_eq!(m.status, StatusCode::OK, "{}", m.text);
    let n = e
        .send(
            "POST",
            "/api/git/repos/1/pulls",
            json!({"title": "Nouvelle", "head": "feature"}),
        )
        .await;
    assert_eq!(n.status, StatusCode::OK, "{}", n.text);
    assert_eq!(n.body["base"], "main");
    let seen = seen.0.lock().unwrap().clone();
    assert!(seen.iter().all(|(_, a, _)| a == "Bearer ghp_jeton"), "{seen:?}");
    let merge = seen.iter().find(|s| s.0 == "merge").unwrap();
    assert_eq!(merge.2["merge_method"], "squash");
    let create = seen.iter().find(|s| s.0 == "create").unwrap();
    assert_eq!(create.2["base"], "main");
}

#[tokio::test]
async fn gitea_pull_requests_and_missing_token() {
    let seen = Seen::default();
    let base = fake_forge(seen.clone()).await;
    let e = env_cfg("", |_| {}).await;
    make_git_topic(&e, Some(&format!("{base}/o/r.git")));
    // Sans GITEA_URL : forge inconnue.
    assert_eq!(e.get("/api/git/repos/1").await.body["forge"], Value::Null);
    let r = e.get("/api/git/repos/1/pulls").await;
    assert_eq!(r.body["error"], "no_forge", "{}", r.text);
    put_secret(&e, "GITEA_URL", &base).await;
    assert_eq!(e.get("/api/git/repos/1").await.body["forge"], "gitea");
    // Lecture sans jeton permise, écriture refusée.
    assert_eq!(e.get("/api/git/repos/1/pulls").await.status, StatusCode::OK);
    let c = e
        .send("POST", "/api/git/repos/1/pulls/7/comments", json!({"body": "x"}))
        .await;
    assert_eq!(c.body["error"], "no_token", "{}", c.text);
    put_secret(&e, "GITEA_TOKEN", "tea_jeton").await;
    let d = e.get("/api/git/repos/1/pulls/7").await;
    assert!(d.body["diff"].as_str().unwrap().contains("+deux"), "{}", d.text);
    let m = e.send("POST", "/api/git/repos/1/pulls/7/merge", json!({})).await;
    assert_eq!(m.status, StatusCode::OK, "{}", m.text);
    let seen = seen.0.lock().unwrap().clone();
    let merge = seen.iter().find(|s| s.0 == "merge").unwrap();
    assert_eq!(merge.1, "token tea_jeton");
    assert_eq!(merge.2["Do"], "merge");
}

#[tokio::test]
async fn session_commit_and_push() {
    let e = env_cfg("", |_| {}).await;
    let up = e.dir.path().join("up.git");
    std::fs::create_dir_all(&up).unwrap();
    git_in(&up, &["init", "-q", "--bare", "-b", "main"]);
    let topic = make_git_topic(&e, Some(&up.display().to_string()));
    let sid = new_session(&e, "auto").await;
    let s = e.get(&format!("/api/sessions/{sid}/git")).await;
    assert_eq!(s.status, StatusCode::OK, "{}", s.text);
    assert_eq!(s.body["git"], true);
    assert_eq!(s.body["branch"], "main");
    std::fs::write(topic.join("nouveau.txt"), "x\n").unwrap();
    let c = e
        .send(
            "POST",
            &format!("/api/sessions/{sid}/git/commit"),
            json!({"message": "Travail de la session"}),
        )
        .await;
    assert_eq!(c.status, StatusCode::OK, "{}", c.text);
    assert_eq!(git_in(&topic, &["log", "-1", "--format=%s"]), "Travail de la session");
    let again = e
        .send(
            "POST",
            &format!("/api/sessions/{sid}/git/commit"),
            json!({"message": "rien"}),
        )
        .await;
    assert_eq!(again.body["error"], "nothing_to_commit");
    let p = e
        .send("POST", &format!("/api/sessions/{sid}/git/push"), json!({}))
        .await;
    assert_eq!(p.status, StatusCode::OK, "{}", p.text);
    assert_eq!(
        git_in(&up, &["rev-parse", "main"]),
        git_in(&topic, &["rev-parse", "HEAD"])
    );
    let s = e.get(&format!("/api/sessions/{sid}/git")).await;
    assert_eq!(s.body["ahead"], 0, "{}", s.text);
    // Pas de forge pour un chemin local : pull request refusée proprement.
    let pr = e
        .send(
            "POST",
            &format!("/api/sessions/{sid}/git/pull-request"),
            json!({"title": "x"}),
        )
        .await;
    assert_eq!(pr.body["error"], "no_forge", "{}", pr.text);
}
