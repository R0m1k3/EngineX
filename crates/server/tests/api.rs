//! Tests d'intégration de l'API (routeur appelé en mémoire).

use std::{net::SocketAddr, path::PathBuf};

use axum::{
    Router,
    body::Body,
    extract::connect_info::MockConnectInfo,
    http::{Request, StatusCode, header},
};
use enginex_server::{AppState, Config, app, db};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery";

struct Env {
    state: AppState,
    _dir: tempfile::TempDir,
}

async fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-llama-server.sh");
    let config = Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        data_dir: dir.path().to_path_buf(),
        llama_bin: Some(fixture.clone()),
        image_bin: fixture,
        nvidia_smi: "/nonexistent/nvidia-smi".into(),
        trust_proxy: false,
        models_dir: dir.path().join("models"),
        llama_port: 0,
        registry: "http://127.0.0.1:9".into(),
        hf_base: "http://127.0.0.1:9".into(),
        timings: Default::default(),
        sandbox: Default::default(),
    };
    let pool = db::open(&config.data_dir).await.unwrap();
    let state = AppState::new(config, pool);
    enginex_server::auth::ensure_setup_code(&state).await.unwrap();
    Env { state, _dir: dir }
}

fn router(state: &AppState, ip: [u8; 4]) -> Router {
    app(state.clone()).layer(MockConnectInfo(SocketAddr::from((ip, 1234))))
}

struct Resp {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Value,
    text: String,
}

async fn call(
    r: &Router,
    method: &str,
    path: &str,
    body: Option<Value>,
    cookie: Option<&str>,
    extra: &[(&str, &str)],
) -> Resp {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "enginex:8090");
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    for (k, v) in extra {
        b = b.header(*k, *v);
    }
    let req = match body {
        Some(v) => b
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(v.to_string())),
        None => b.body(Body::empty()),
    }
    .unwrap();
    let res = r.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let body = serde_json::from_str(&text).unwrap_or(Value::Null);
    Resp {
        status,
        headers,
        body,
        text,
    }
}

fn cookie_of(r: &Resp) -> String {
    let sc = r.headers.get(header::SET_COOKIE).expect("Set-Cookie").to_str().unwrap();
    sc.split(';').next().unwrap().to_string()
}

async fn do_setup(e: &Env, r: &Router) -> String {
    let code = e.state.setup_code().unwrap();
    let res = call(
        r,
        "POST",
        "/api/auth/setup",
        Some(json!({"code": code, "password": PASSWORD})),
        None,
        &[],
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.text);
    cookie_of(&res)
}

async fn do_login(r: &Router, password: &str) -> Resp {
    call(
        r,
        "POST",
        "/api/auth/login",
        Some(json!({"password": password})),
        None,
        &[],
    )
    .await
}

#[tokio::test]
async fn health_is_public() {
    let e = env().await;
    let res = call(&router(&e.state, [10, 0, 0, 1]), "GET", "/api/health", None, None, &[]).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.body["ok"], true);
    assert_eq!(res.headers.get("x-content-type-options").unwrap(), "nosniff");
}

#[tokio::test]
async fn fresh_db_requires_setup() {
    let e = env().await;
    let res = call(
        &router(&e.state, [10, 0, 0, 1]),
        "GET",
        "/api/auth/state",
        None,
        None,
        &[],
    )
    .await;
    assert_eq!(res.body, json!({"setup_required": true, "authenticated": false}));
    let code = e.state.setup_code().unwrap();
    assert_eq!(code.replace('-', "").len(), 12);
}

#[tokio::test]
async fn setup_flow() {
    let e = env().await;
    let r = router(&e.state, [10, 0, 0, 1]);
    let bad = call(
        &r,
        "POST",
        "/api/auth/setup",
        Some(json!({"code": "AAAA-AAAA-AAAA", "password": PASSWORD})),
        None,
        &[],
    )
    .await;
    assert_eq!(
        (bad.status, bad.body["error"].as_str()),
        (StatusCode::FORBIDDEN, Some("bad_setup_code"))
    );
    let code = e.state.setup_code().unwrap();
    let short = call(
        &r,
        "POST",
        "/api/auth/setup",
        Some(json!({"code": code.to_lowercase(), "password": "12345678901"})),
        None,
        &[],
    )
    .await;
    assert_eq!(
        (short.status, short.body["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some("password_too_short"))
    );
    let ok = call(
        &r,
        "POST",
        "/api/auth/setup",
        Some(json!({"code": code, "password": PASSWORD})),
        None,
        &[],
    )
    .await;
    assert_eq!(ok.status, StatusCode::OK);
    let sc = ok.headers.get(header::SET_COOKIE).unwrap().to_str().unwrap();
    assert!(
        sc.contains("HttpOnly") && sc.contains("SameSite=Strict") && sc.contains("Max-Age=2592000"),
        "{sc}"
    );
    assert!(!sc.contains("Secure"));
    assert!(e.state.setup_code().is_none());
    let again = call(
        &r,
        "POST",
        "/api/auth/setup",
        Some(json!({"code": code, "password": PASSWORD})),
        None,
        &[],
    )
    .await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    let state = call(&r, "GET", "/api/auth/state", None, Some(&cookie_of(&ok)), &[]).await;
    assert_eq!(state.body, json!({"setup_required": false, "authenticated": true}));
}

#[tokio::test]
async fn bad_json_is_bad_request() {
    let e = env().await;
    let r = router(&e.state, [10, 0, 0, 1]);
    let res = call(&r, "POST", "/api/auth/login", Some(json!({"nope": 1})), None, &[]).await;
    assert_eq!(
        (res.status, res.body["error"].as_str()),
        (StatusCode::BAD_REQUEST, Some("bad_request"))
    );
}

#[tokio::test]
async fn login_rate_limit() {
    let e = env().await;
    let a = router(&e.state, [10, 0, 0, 1]);
    do_setup(&e, &a).await;
    for _ in 0..5 {
        assert_eq!(do_login(&a, "wrong password!!").await.status, StatusCode::UNAUTHORIZED);
    }
    let locked = do_login(&a, PASSWORD).await;
    assert_eq!(locked.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(locked.headers.get(header::RETRY_AFTER).is_some());
    let b = router(&e.state, [10, 0, 0, 2]);
    assert_eq!(do_login(&b, PASSWORD).await.status, StatusCode::OK);
    // Un succès remet le compteur à zéro.
    let c = router(&e.state, [10, 0, 0, 3]);
    for _ in 0..4 {
        assert_eq!(do_login(&c, "wrong password!!").await.status, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(do_login(&c, PASSWORD).await.status, StatusCode::OK);
    for _ in 0..5 {
        assert_eq!(do_login(&c, "wrong password!!").await.status, StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn system_requires_session() {
    let e = env().await;
    let r = router(&e.state, [10, 0, 0, 1]);
    let cookie = do_setup(&e, &r).await;
    assert_eq!(
        call(&r, "GET", "/api/system", None, None, &[]).await.status,
        StatusCode::UNAUTHORIZED
    );
    let res = call(&r, "GET", "/api/system", None, Some(&cookie), &[]).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.text);
    assert_eq!(res.body["llama"]["version"], "7000 (abcdef0)");
    let devs = res.body["llama"]["devices"].as_array().unwrap();
    assert_eq!(devs.len(), 2);
    assert_eq!(devs[1]["name"], "NVIDIA GeForce RTX 3060");
    assert_eq!(res.body["gpus"], json!([]));
    assert_eq!(res.body["gpus_error"], "not_found");
}

#[tokio::test]
async fn logout_invalidates() {
    let e = env().await;
    let r = router(&e.state, [10, 0, 0, 1]);
    let cookie = do_setup(&e, &r).await;
    let out = call(&r, "POST", "/api/auth/logout", None, Some(&cookie), &[]).await;
    assert_eq!(out.status, StatusCode::OK);
    assert!(
        out.headers
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    assert_eq!(
        call(&r, "GET", "/api/system", None, Some(&cookie), &[]).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn password_change_closes_other_sessions() {
    let e = env().await;
    let r = router(&e.state, [10, 0, 0, 1]);
    let a = do_setup(&e, &r).await;
    let b = cookie_of(&do_login(&r, PASSWORD).await);
    let bad = call(
        &r,
        "POST",
        "/api/auth/password",
        Some(json!({"current": "nope nope nope", "new": "another long password"})),
        Some(&a),
        &[],
    )
    .await;
    assert_eq!(bad.status, StatusCode::UNAUTHORIZED);
    let ok = call(
        &r,
        "POST",
        "/api/auth/password",
        Some(json!({"current": PASSWORD, "new": "another long password"})),
        Some(&a),
        &[],
    )
    .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text);
    assert_eq!(
        call(&r, "GET", "/api/system", None, Some(&b), &[]).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        call(&r, "GET", "/api/system", None, Some(&a), &[]).await.status,
        StatusCode::OK
    );
    assert_eq!(do_login(&r, PASSWORD).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(do_login(&r, "another long password").await.status, StatusCode::OK);
}

#[tokio::test]
async fn origin_check() {
    let e = env().await;
    let r = router(&e.state, [10, 0, 0, 1]);
    do_setup(&e, &r).await;
    let body = || Some(json!({"password": PASSWORD}));
    let evil = call(
        &r,
        "POST",
        "/api/auth/login",
        body(),
        None,
        &[("origin", "http://evil.example")],
    )
    .await;
    assert_eq!(
        (evil.status, evil.body["error"].as_str()),
        (StatusCode::FORBIDDEN, Some("forbidden_origin"))
    );
    let null = call(&r, "POST", "/api/auth/login", body(), None, &[("origin", "null")]).await;
    assert_eq!(null.status, StatusCode::FORBIDDEN);
    let same = call(
        &r,
        "POST",
        "/api/auth/login",
        body(),
        None,
        &[("origin", "https://enginex:8090")],
    )
    .await;
    assert_eq!(same.status, StatusCode::OK);
    let none = call(&r, "POST", "/api/auth/login", body(), None, &[]).await;
    assert_eq!(none.status, StatusCode::OK);
}

#[tokio::test]
async fn expired_session() {
    let e = env().await;
    let r = router(&e.state, [10, 0, 0, 1]);
    let cookie = do_setup(&e, &r).await;
    sqlx::query("UPDATE sessions SET expires_at = 1")
        .execute(e.state.db())
        .await
        .unwrap();
    assert_eq!(
        call(&r, "GET", "/api/system", None, Some(&cookie), &[]).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(do_login(&r, PASSWORD).await.status, StatusCode::OK);
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE expires_at = 1")
        .fetch_one(e.state.db())
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn ui_and_unknown_routes() {
    let e = env().await;
    let r = router(&e.state, [10, 0, 0, 1]);
    let root = call(&r, "GET", "/", None, None, &[]).await;
    assert_eq!(root.status, StatusCode::OK);
    assert!(
        root.headers
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    let api = call(&r, "GET", "/api/nope", None, None, &[]).await;
    assert_eq!(
        (api.status, api.body.clone()),
        (StatusCode::NOT_FOUND, json!({"error": "not_found"}))
    );
    let post = call(&r, "POST", "/", None, None, &[]).await;
    assert_eq!(
        (post.status, post.body.clone()),
        (StatusCode::METHOD_NOT_ALLOWED, json!({"error": "method_not_allowed"}))
    );
    let missing = call(&r, "GET", "/absent.png", None, None, &[]).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn reset_then_setup_again() {
    let e = env().await;
    let r = router(&e.state, [10, 0, 0, 1]);
    do_setup(&e, &r).await;
    sqlx::query("DELETE FROM settings WHERE key = 'password_hash'")
        .execute(e.state.db())
        .await
        .unwrap();
    // Le code est recréé à la demande, sans redémarrage.
    let st = call(&r, "GET", "/api/auth/state", None, None, &[]).await;
    assert_eq!(st.body["setup_required"], true);
    assert!(e.state.setup_code().is_some());
    do_setup(&e, &r).await;
}
