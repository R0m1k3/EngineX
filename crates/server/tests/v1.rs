//! API `/v1` : clés, relais Responses → chat/completions avec le faux
//! llama-server scripté, porte de service pendant les chargements.

mod common;

use std::time::Duration;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use common::{Env, call, env, env_with, make_active_preset, new_key, publish, serve_tcp};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

struct V1Resp {
    status: StatusCode,
    text: String,
    body: Value,
}

async fn v1(
    r: &Router,
    method: &str,
    path: &str,
    key: Option<&str>,
    body: Option<Value>,
    origin: Option<&str>,
) -> V1Resp {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "enginex:8090");
    if let Some(k) = key {
        b = b.header(header::AUTHORIZATION, format!("Bearer {k}"));
    }
    if let Some(o) = origin {
        b = b.header(header::ORIGIN, o);
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
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    V1Resp {
        status,
        body: serde_json::from_str(&text).unwrap_or(Value::Null),
        text,
    }
}

fn request(text: &str) -> Value {
    json!({"model": "x", "stream": true, "store": false,
        "input": [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": text}]}]})
}

/// Événements SSE : (nom, données).
fn events(text: &str) -> Vec<(String, Value)> {
    text.split("\n\n")
        .filter(|b| !b.trim().is_empty())
        .map(|b| {
            let mut name = String::new();
            let mut data = Value::Null;
            for l in b.lines() {
                if let Some(n) = l.strip_prefix("event: ") {
                    name = n.to_string();
                } else if let Some(d) = l.strip_prefix("data: ") {
                    data = serde_json::from_str(d).unwrap();
                }
            }
            (name, data)
        })
        .collect()
}

fn names(ev: &[(String, Value)]) -> Vec<&str> {
    ev.iter().map(|(n, _)| n.as_str()).collect()
}

fn chat_log(path: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

async fn job_done(e: &Env, kind: &str) -> Value {
    e.until("/api/engine/versions", |v| {
        v["job"]["kind"] == kind && v["job"]["finished"] == true
    })
    .await["job"]
        .clone()
}

#[tokio::test]
async fn keys_limiter_origin_and_errors() {
    let e = env().await;
    let r = &e.router;
    // Gestion des clés.
    let bad = e.send("POST", "/api/api-keys", json!({"name": "  "})).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    let key = new_key(&e, "codex").await;
    assert!(key.starts_with("ex_") && key.len() > 40);
    let dup = e.send("POST", "/api/api-keys", json!({"name": "codex"})).await;
    assert_eq!(
        (dup.status, dup.body["error"].as_str()),
        (StatusCode::CONFLICT, Some("name_taken"))
    );
    let list = e.get("/api/api-keys").await;
    assert_eq!(list.body[0]["name"], "codex");
    assert!(list.body[0].get("key").is_none() && !list.text.contains(&key));
    assert!(list.body[0]["last_used_at"].is_null());

    // Sans clé, clé inconnue, cookie seul → 401 au format OpenAI.
    let res = v1(r, "GET", "/v1/models", None, None, None).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);
    assert_eq!(res.body["error"]["code"], "invalid_api_key");
    assert_eq!(res.body["error"]["type"], "authentication_error");
    assert_eq!(
        v1(r, "GET", "/v1/models", Some("ex_inconnue"), None, None).await.status,
        StatusCode::UNAUTHORIZED
    );
    let res = call(r, "GET", "/v1/models", None, Some(&e.cookie)).await;
    assert_eq!(res.status, StatusCode::UNAUTHORIZED);

    // Bonne clé → 200, last_used_at posé ; Origin étrangère acceptée.
    let res = v1(r, "GET", "/v1/models", Some(&key), None, None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.text);
    assert_eq!(res.body, json!({"object": "list", "data": []}));
    assert!(e.get("/api/api-keys").await.body[0]["last_used_at"].is_i64());
    let res = v1(
        r,
        "POST",
        "/v1/responses",
        Some(&key),
        Some(request("hi")),
        Some("https://ailleurs.example"),
    )
    .await;
    assert_ne!(res.status, StatusCode::FORBIDDEN);
    // Aucun preset actif → 503 ; stream absent → 400 (avant le 503).
    assert_eq!(res.status, StatusCode::SERVICE_UNAVAILABLE, "{}", res.text);
    assert_eq!(res.body["error"]["code"], "engine_unavailable");
    let mut nostream = request("hi");
    nostream["stream"] = json!(false);
    let res = v1(r, "POST", "/v1/responses", Some(&key), Some(nostream), None).await;
    assert_eq!(
        (res.status, res.body["error"]["code"].as_str()),
        (StatusCode::BAD_REQUEST, Some("stream_required"))
    );
    let res = v1(
        r,
        "POST",
        "/v1/responses",
        Some(&key),
        Some(json!("pas un objet")),
        None,
    )
    .await;
    assert_eq!(res.body["error"]["code"], "invalid_request_error");
    // Route inconnue.
    let res = v1(r, "GET", "/v1/inconnue", Some(&key), None, None).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.body["error"]["code"], "not_found");
    // Configuration Codex : 409 sans preset actif.
    let res = e.get("/api/codex-config").await;
    assert_eq!(res.body["error"], "no_active_preset");

    // Clé révoquée.
    let id = e.get("/api/api-keys").await.body[0]["id"].as_i64().unwrap();
    assert_eq!(
        e.send("DELETE", &format!("/api/api-keys/{id}"), json!({})).await.status,
        StatusCode::OK
    );
    assert_eq!(
        v1(r, "GET", "/v1/models", Some(&key), None, None).await.status,
        StatusCode::UNAUTHORIZED
    );

    // 20 échecs depuis une IP → 429 sur /v1 ; la connexion à l'UI marche encore.
    for _ in 0..19 {
        v1(r, "GET", "/v1/models", Some("ex_faux"), None, None).await;
    }
    let res = v1(r, "GET", "/v1/models", Some("ex_faux"), None, None).await;
    assert_eq!(res.status, StatusCode::TOO_MANY_REQUESTS, "{}", res.text);
    assert_eq!(res.body["error"]["type"], "rate_limit_error");
    let login = call(
        r,
        "POST",
        "/api/auth/login",
        Some(json!({"password": common::PASSWORD})),
        None,
    )
    .await;
    assert_eq!(login.status, StatusCode::OK, "{}", login.text);
}

#[tokio::test]
async fn relay_nominal_context_chat_models_and_config() {
    let tmp = tempfile::tempdir().unwrap();
    let script = tmp.path().join("script.json");
    std::fs::write(
        &script,
        json!([
            {"reasoning": "Je réfléchis.", "content": "Bonjour !"},
            {"tool_calls": [{"name": "mcp__test__echo", "arguments": "{\"text\":\"hi\"}", "id": "call_1"}]},
        ])
        .to_string(),
    )
    .unwrap();
    let log = tmp.path().join("chat.log");
    let e = env_with(&format!(
        "FAKE_CHAT_SCRIPT={} FAKE_CHAT_LOG={} FAKE_CTX_LIMIT=3000",
        script.display(),
        log.display()
    ))
    .await;
    let id = make_active_preset(&e).await;
    let key = new_key(&e, "test").await;
    let r = &e.router;
    let preset = e.get("/api/presets").await.body[0].clone();
    let name = preset["name"].as_str().unwrap().to_string();
    let temp = preset["settings"]["sampling"]["temperature"].clone();
    assert!(temp.is_number());

    // Flux nominal : raisonnement puis texte.
    let res = v1(r, "POST", "/v1/responses", Some(&key), Some(request("salut")), None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.text);
    let ev = events(&res.text);
    let n = names(&ev);
    assert_eq!(&n[..2], ["response.created", "response.in_progress"]);
    assert_eq!(*n.last().unwrap(), "response.completed", "{n:?}");
    let out = &ev.last().unwrap().1["response"]["output"];
    assert_eq!(out[0]["type"], "reasoning");
    assert_eq!(out[1]["content"][0]["text"], "Bonjour !");
    assert_eq!(ev.last().unwrap().1["response"]["model"], name);
    let sent = chat_log(&log);
    assert_eq!(sent[0]["temperature"], temp, "échantillonnage du preset injecté");
    assert_eq!(sent[0]["model"], name);
    assert_eq!(sent[0]["messages"][0], json!({"role": "user", "content": "salut"}));

    // Appel d'outil à plat → name + namespace (l'outil est déclaré).
    let mut req = request("outil");
    req["tools"] = json!([{"type": "namespace", "name": "mcp__test", "description": "t",
        "tools": [{"type": "function", "name": "echo", "parameters": {"type": "object"}}]}]);
    let res = v1(r, "POST", "/v1/responses", Some(&key), Some(req), None).await;
    let ev = events(&res.text);
    let item = &ev.last().unwrap().1["response"]["output"][0];
    assert_eq!(
        (item["name"].as_str(), item["namespace"].as_str()),
        (Some("echo"), Some("mcp__test")),
        "{item}"
    );
    assert_eq!(chat_log(&log)[1]["tools"][0]["function"]["name"], "mcp__test__echo");

    // Échantillonnage modifié : appliqué à la requête suivante, sans rechargement.
    let pid = e.get("/api/engine").await.body["state"]["pid"].clone();
    let mut s = preset["settings"].clone();
    s["sampling"]["temperature"] = json!(0.25);
    let up = e
        .send(
            "PUT",
            &format!("/api/presets/{id}"),
            json!({"name": name, "settings": s}),
        )
        .await;
    assert_eq!(up.body["reload_needed"], false);
    v1(r, "POST", "/v1/responses", Some(&key), Some(request("encore")), None).await;
    assert_eq!(chat_log(&log)[2]["temperature"], 0.25);
    assert_eq!(e.get("/api/engine").await.body["state"]["pid"], pid);

    // Contexte dépassé → response.failed context_length_exceeded.
    let res = v1(
        r,
        "POST",
        "/v1/responses",
        Some(&key),
        Some(request(&"x".repeat(4000))),
        None,
    )
    .await;
    let ev = events(&res.text);
    assert_eq!(ev.last().unwrap().0, "response.failed");
    assert_eq!(
        ev.last().unwrap().1["response"]["error"]["code"],
        "context_length_exceeded"
    );

    // /v1/chat/completions non streamé et streamé.
    let body = json!({"model": "autre", "temperature": 1.5, "top_k": 99,
        "messages": [{"role": "user", "content": "hi"}]});
    let res = v1(r, "POST", "/v1/chat/completions", Some(&key), Some(body.clone()), None).await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.text);
    assert_eq!(res.body["object"], "chat.completion");
    let sent = chat_log(&log).last().unwrap().clone();
    assert_eq!(
        (sent["model"].as_str(), sent["temperature"].as_f64()),
        (Some(name.as_str()), Some(0.25))
    );
    let top_k = &preset["settings"]["sampling"]["top_k"];
    if top_k.is_null() {
        assert_eq!(sent["top_k"], 99);
    } else {
        assert_eq!(sent["top_k"].as_f64(), top_k.as_f64());
    }
    let mut sbody = body;
    sbody["stream"] = json!(true);
    let res = v1(r, "POST", "/v1/chat/completions", Some(&key), Some(sbody), None).await;
    assert!(res.text.contains("data: [DONE]"), "{}", res.text);

    // /v1/models = preset actif.
    let res = v1(r, "GET", "/v1/models", Some(&key), None, None).await;
    assert_eq!(res.body["data"][0]["id"], name);
    assert_eq!(res.body["data"][0]["context_window"], preset["settings"]["ctx"]);

    // Configuration Codex : nom, contexte, URL de base (avec et sans proxy).
    let c = e.get("/api/codex-config").await;
    assert_eq!(c.body["base_url"], "http://enginex:8090/v1", "{}", c.text);
    let toml = c.body["toml"].as_str().unwrap();
    assert!(toml.contains(&format!("model = \"{name}\"")), "{toml}");
    assert!(
        toml.contains(&format!("model_context_window = {}", preset["settings"]["ctx"])),
        "{toml}"
    );
    assert!(toml.contains("base_url = \"http://enginex:8090/v1\""), "{toml}");
    assert!(toml.contains("env_key = \"ENGINEX_API_KEY\""));

    // Porte libre et dernière activité posée.
    assert!(e.state.serve_gate().try_write().is_ok());
    assert!(e.state.last_done_ms() > 0);
}

#[tokio::test]
async fn codex_config_behind_proxy() {
    // Même lecture que le cookie Secure : X-Forwarded-* seulement si trust_proxy.
    for (trust, want) in [(false, "http://enginex:8090/v1"), (true, "https://ex.example.org/v1")] {
        let e = common::env_full("", trust).await;
        make_active_preset(&e).await;
        assert_eq!(proxied_config(&e).await["base_url"], want, "trust_proxy = {trust}");
    }
}

async fn proxied_config(e: &Env) -> Value {
    let req = Request::builder()
        .uri("/api/codex-config")
        .header(header::HOST, "enginex:8090")
        .header(header::COOKIE, &e.cookie)
        .header("x-forwarded-proto", "https")
        .header("x-forwarded-host", "ex.example.org")
        .body(Body::empty())
        .unwrap();
    let res = e.router.clone().oneshot(req).await.unwrap();
    serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

#[tokio::test]
async fn waiting_during_activation_and_disconnect() {
    let tmp = tempfile::tempdir().unwrap();
    let script = tmp.path().join("script.json");
    std::fs::write(
        &script,
        json!([{"content": "un texte assez long pour plusieurs morceaux"}]).to_string(),
    )
    .unwrap();
    let e = env_with(&format!(
        "FAKE_LOAD_MS=1500 FAKE_CHUNK_DELAY_MS=150 FAKE_CHAT_SCRIPT={}",
        script.display()
    ))
    .await;
    let id = make_active_preset(&e).await;
    let key = new_key(&e, "test").await;

    // Rechargement en cours (1,5 s) : flux ouvert tout de suite, maintiens,
    // puis réponse complète.
    let r = e.send("POST", &format!("/api/presets/{id}/activate"), json!({})).await;
    assert_eq!(r.body["started"], true, "{}", r.text);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let res = v1(
        &e.router,
        "POST",
        "/v1/responses",
        Some(&key),
        Some(request("attends")),
        None,
    )
    .await;
    let ev = events(&res.text);
    let n = names(&ev);
    assert!(n.iter().filter(|x| **x == "response.in_progress").count() >= 3, "{n:?}");
    assert_eq!(*n.last().unwrap(), "response.completed", "{n:?}");
    job_done(&e, "activate").await;

    // Client qui coupe au milieu du flux : porte rendue, last_done posé.
    let before = e.state.last_done_ms();
    let req = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header(header::HOST, "enginex:8090")
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(request("coupe").to_string()))
        .unwrap();
    let res = e.router.clone().oneshot(req).await.unwrap();
    let mut body = res.into_body();
    let mut seen = String::new();
    while !seen.contains("output_text.delta") {
        let f = body.frame().await.unwrap().unwrap();
        seen.push_str(&String::from_utf8_lossy(f.data_ref().unwrap()));
    }
    assert!(e.state.serve_gate().try_write().is_err(), "porte tenue pendant le flux");
    drop(body);
    for _ in 0..50 {
        if e.state.serve_gate().try_write().is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        e.state.serve_gate().try_write().is_ok(),
        "porte rendue après la déconnexion"
    );
    assert!(e.state.last_done_ms() > before);
}

#[tokio::test]
async fn gate_holds_loads_until_requests_end() {
    let tmp = tempfile::tempdir().unwrap();
    let script = tmp.path().join("script.json");
    // ~30 morceaux à 100 ms : flux de ~3 s.
    std::fs::write(&script, json!([{"content": "x".repeat(110)}]).to_string()).unwrap();
    let e = env_with(&format!(
        "FAKE_CHUNK_DELAY_MS=100 FAKE_CHAT_SCRIPT={}",
        script.display()
    ))
    .await;
    let id = make_active_preset(&e).await;
    let key = new_key(&e, "test").await;
    let base = serve_tcp(e.router.clone()).await;
    let http = reqwest::Client::new();
    let post = |text: &'static str| {
        let (http, base, key) = (http.clone(), base.clone(), key.clone());
        tokio::spawn(async move {
            let t0 = std::time::Instant::now();
            let r = http
                .post(format!("{base}/v1/responses"))
                .bearer_auth(key)
                .json(&request(text))
                .send()
                .await
                .unwrap();
            (r.text().await.unwrap(), t0.elapsed())
        })
    };

    // Porte 4 : pendant le téléchargement d'une version, une requête passe.
    publish(
        &e.reg,
        "b12500",
        &format!("FAKE_CHUNK_DELAY_MS=100 FAKE_CHAT_SCRIPT={}", script.display()),
    );
    e.reg.lock().unwrap().blob_delay_ms = 4000;
    e.send("POST", "/api/engine/versions/check", json!({})).await;
    let r = e.send("POST", "/api/engine/versions/install", json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    e.until("/api/engine/versions", |v| v["job"]["step"] == "download")
        .await;
    let (text, _) = post("pendant le téléchargement").await.unwrap();
    assert!(text.contains("response.completed"), "{text}");
    let v = e.get("/api/engine/versions").await.body;
    assert_eq!(
        v["job"]["step"], "download",
        "la requête est passée pendant le téléchargement"
    );
    job_done(&e, "install").await;
    e.reg.lock().unwrap().blob_delay_ms = 0;

    // Porte 1 et 2 : flux lent en cours + bascule manuelle → la bascule
    // attend ; une requête arrivée pendant l'attente passe après elle.
    let slow = post("lent");
    tokio::time::sleep(Duration::from_millis(400)).await;
    let r = e.send("POST", "/api/engine/versions/image/use", json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    e.until("/api/engine/versions", |v| v["job"]["step"] == "wait_requests")
        .await;
    let late = post("arrivée pendant l'attente");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(e.get("/api/engine/versions").await.body["job"]["step"], "wait_requests");
    let (text, _) = slow.await.unwrap();
    assert!(
        text.trim_end().ends_with('}') && text.contains("event: response.completed"),
        "{text}"
    );
    let job = job_done(&e, "use").await;
    assert_eq!(job["step"], "done", "{job}");
    let (text, _) = late.await.unwrap();
    assert!(text.contains("event: response.completed"), "{text}");
    assert_eq!(e.get("/api/engine/versions").await.body["active"], "image");

    // Porte 3 : même scénario avec une activation de preset.
    let slow = post("lent 2");
    tokio::time::sleep(Duration::from_millis(400)).await;
    let r = e.send("POST", &format!("/api/presets/{id}/activate"), json!({})).await;
    assert_eq!(r.body["started"], true);
    e.until("/api/engine/versions", |v| v["job"]["step"] == "wait_requests")
        .await;
    let late = post("pendant l'activation");
    let (text, _) = slow.await.unwrap();
    assert!(text.contains("event: response.completed"), "{text}");
    assert_eq!(job_done(&e, "activate").await["step"], "done");
    let (text, _) = late.await.unwrap();
    assert!(text.contains("event: response.completed"), "{text}");

    // « Forcer » : rien à forcer hors attente ; pendant l'attente, le flux
    // en cours est coupé (réponse en échec) et l'activation passe aussitôt.
    let r = e.send("POST", "/api/engine/force", json!({})).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "{}", r.text);
    let slow = post("lent 3");
    tokio::time::sleep(Duration::from_millis(400)).await;
    let r = e.send("POST", &format!("/api/presets/{id}/activate"), json!({})).await;
    assert_eq!(r.body["started"], true);
    e.until("/api/engine/versions", |v| v["job"]["step"] == "wait_requests")
        .await;
    let r = e.send("POST", "/api/engine/force", json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let (text, took) = slow.await.unwrap();
    assert!(
        text.contains("event: response.failed") && text.contains("bascule forcée"),
        "{text}"
    );
    assert!(took < Duration::from_secs(5), "flux coupé tard : {took:?}");
    assert_eq!(job_done(&e, "activate").await["step"], "done");
}

// ---- Étape 4a : masquage et clés internes ----

#[tokio::test]
async fn secrets_are_masked_before_the_model_even_with_quotes_and_backslashes() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("chat.log");
    let e = env_with(&format!("FAKE_CHAT_LOG={}", log.display())).await;
    make_active_preset(&e).await;
    let key = new_key(&e, "codex").await;
    let tricky = r#"ab"cd\ef-9876"#;
    for (name, value) in [("PLAIN_TOKEN", "ghp_plainvalue123"), ("TRICKY", tricky)] {
        let r = e
            .send(
                "PUT",
                "/api/vault",
                json!({"scope": "global", "name": name, "value": value}),
            )
            .await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    }
    let body = json!({"model": "x", "stream": true, "store": false, "input": [
        {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "lis le fichier"}]},
        {"type": "function_call", "call_id": "c1", "name": "exec_command", "arguments": "{\"cmd\":\"cat .env\"}"},
        {"type": "function_call_output", "call_id": "c1",
         "output": format!("TOKEN=ghp_plainvalue123\nOTHER={tricky}\n")},
    ]});
    let r = v1(&e.router, "POST", "/v1/responses", Some(&key), Some(body), None).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let sent = chat_log(&log);
    let raw = serde_json::to_string(&sent).unwrap();
    assert!(!raw.contains("ghp_plainvalue123"), "{raw}");
    assert!(!sent.last().unwrap().to_string().contains("9876"), "{raw}");
    let tool = sent.last().unwrap()["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "tool")
        .unwrap()
        .clone();
    assert_eq!(tool["content"], "TOKEN=***\nOTHER=***\n");
    // Même chose sur /v1/chat/completions.
    let chat = json!({"model": "x", "messages": [{"role": "user", "content": "x ghp_plainvalue123 y"}]});
    let r = v1(&e.router, "POST", "/v1/chat/completions", Some(&key), Some(chat), None).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(chat_log(&log).last().unwrap()["messages"][0]["content"], "x *** y");
}

#[tokio::test]
async fn internal_keys_are_scoped_and_hidden() {
    use sha2::{Digest, Sha256};
    let e = env().await;
    make_active_preset(&e).await;
    let key = "ex_internal-key-for-tests";
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO api_keys (name, hash, created_at, internal) VALUES ('sandbox:t', ?, 0, 1) RETURNING id",
    )
    .bind(Sha256::digest(key.as_bytes()).to_vec())
    .fetch_one(e.state.db())
    .await
    .unwrap();
    let chat = json!({"model": "x", "messages": [{"role": "user", "content": "x"}]});
    let r = v1(&e.router, "POST", "/v1/chat/completions", Some(key), Some(chat), None).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "{}", r.text);
    assert_eq!(r.body["error"]["code"], "key_scope");
    assert_eq!(r.body["error"]["type"], "invalid_request_error");
    let r = v1(
        &e.router,
        "POST",
        "/v1/responses",
        Some(key),
        Some(request("bonjour")),
        None,
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let r = v1(&e.router, "GET", "/v1/models", Some(key), None, None).await;
    assert_eq!(r.status, StatusCode::OK);
    let list = e.get("/api/api-keys").await;
    assert!(!list.text.contains("sandbox:t"), "{}", list.text);
    let r = e.send("DELETE", &format!("/api/api-keys/{id}"), json!({})).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}
