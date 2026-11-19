//! Accélération des presets : MTP détecté dans le GGUF, brouillon, ngram,
//! placement des experts MoE, débit et brouillon accepté relevés par /v1.

mod common;

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use common::{Env, MTP, REPO, env, env_with, make_active_preset, new_key};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn download(e: &Env, repo: &str) -> Value {
    let r = e
        .send("POST", "/api/downloads", json!({"repo": repo, "quant": "Q8_0"}))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    e.until("/api/downloads", |v| {
        v.as_array()
            .unwrap()
            .iter()
            .any(|d| d["repo"] == repo && d["state"] == "done")
    })
    .await;
    let models = e.get("/api/models").await.body;
    let m = models["models"]
        .as_array()
        .or_else(|| models.as_array())
        .unwrap()
        .iter()
        .find(|m| m["repo"] == repo)
        .unwrap()
        .clone();
    let presets = e
        .until("/api/presets", |v| {
            v.as_array().unwrap().iter().any(|p| p["model_id"] == m["id"])
        })
        .await;
    let p = presets
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["model_id"] == m["id"])
        .unwrap()
        .clone();
    json!({"model": m, "preset": p})
}

async fn save(e: &Env, p: &Value, settings: Value) -> StatusCode {
    let id = p["id"].as_i64().unwrap();
    e.send(
        "PUT",
        &format!("/api/presets/{id}"),
        json!({"name": p["name"], "settings": settings}),
    )
    .await
    .status
}

#[tokio::test]
async fn mtp_moe_and_speed() {
    let e = env().await;
    let mtp = download(&e, MTP).await;
    let dense = download(&e, REPO).await;
    assert_eq!(mtp["model"]["meta"]["nextn_predict_layers"], 1);
    assert_eq!(mtp["model"]["meta"]["expert_count"], 8);
    assert!(dense["model"]["meta"]["nextn_predict_layers"].is_null());
    // MTP actif d'office sur le modèle qui en a, pas sur l'autre.
    assert_eq!(mtp["preset"]["settings"]["spec"]["draft"], "mtp");
    assert_eq!(dense["preset"]["settings"]["spec"]["draft"], "none");

    // MTP refusé sur un modèle sans couches MTP ; brouillon = un autre modèle.
    let mut s = dense["preset"]["settings"].clone();
    s["spec"]["draft"] = json!("mtp");
    assert_eq!(save(&e, &dense["preset"], s.clone()).await, StatusCode::BAD_REQUEST);
    s["spec"]["draft"] = json!("model");
    s["spec"]["draft_model_id"] = dense["model"]["id"].clone();
    assert_eq!(save(&e, &dense["preset"], s.clone()).await, StatusCode::BAD_REQUEST);
    s["spec"]["draft_model_id"] = mtp["model"]["id"].clone();
    assert_eq!(save(&e, &dense["preset"], s).await, StatusCode::OK);

    // MTP + ngram + 4 couches d'experts en RAM (ngram alors retiré).
    let mut s = mtp["preset"]["settings"].clone();
    s["spec"]["ngram"] = json!(true);
    s["spec"]["draft_max"] = json!(5);
    s["moe"] = json!({"mode": "layers", "cpu_layers": 4, "cache_mib": 0});
    assert_eq!(save(&e, &mtp["preset"], s).await, StatusCode::OK);
    let id = mtp["preset"]["id"].as_i64().unwrap();
    let r = e.send("POST", &format!("/api/presets/{id}/activate"), json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let eng = e
        .until("/api/engine", |v| {
            v["state"]["state"] == "ready" && v["state"]["preset_id"] == id
        })
        .await;
    assert_eq!(
        eng["caps"],
        json!({"fit": true, "spec": true, "moe": true, "budget": true})
    );
    let logs = e.get("/api/engine/logs").await.body;
    let argv = logs["lines"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .find(|l| l.contains("main: argv"))
        .unwrap()
        .to_string();
    for part in [
        "--spec-type draft-mtp --spec-draft-n-max 5",
        "--n-cpu-moe 4",
        "-ngl 999",
        "--fit off",
    ] {
        assert!(argv.contains(part), "{part} absent de {argv}");
    }
    // ngram retiré : experts en RAM.
    assert!(!argv.contains("ngram"), "{argv}");

    // Une requête /v1 : débit et brouillon accepté relevés.
    let key = new_key(&e, "accel").await;
    let req = Request::builder()
        .method("POST")
        .uri("/v1/responses")
        .header(header::HOST, "enginex:8090")
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"model": "x", "stream": true, "store": false,
                "input": [{"type": "message", "role": "user",
                    "content": [{"type": "input_text", "text": "salut"}]}]})
            .to_string(),
        ))
        .unwrap();
    let res = e.router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let _ = res.into_body().collect().await.unwrap();
    let eng = e.until("/api/engine", |v| v["last_gen"].is_object()).await;
    let g = &eng["last_gen"];
    assert_eq!(g["tokens_per_s"], 50.0);
    assert_eq!(
        (g["draft_n"].as_f64(), g["draft_accepted"].as_f64()),
        (Some(16.0), Some(12.0))
    );
}

/// `--fit` a réduit le contexte faute de VRAM : Codex doit recevoir le contexte
/// réellement chargé, pas celui du preset (sinon débordement puis
/// « max_output_tokens » à vide).
#[tokio::test]
async fn codex_gets_the_loaded_context() {
    let e = env_with("FAKE_FIT_CTX=16384").await;
    let id = make_active_preset(&e).await;
    let presets = e.get("/api/presets").await.body;
    let ctx = presets[0]["settings"]["ctx"].as_u64().unwrap();
    assert!(ctx > 16384, "preset de test trop petit : {ctx}");
    let eng = e.until("/api/engine", |v| v["n_ctx"] == 16384).await;
    assert_eq!(eng["state"]["preset_id"], id);

    let c = e.get("/api/codex-config").await;
    let toml = c.body["toml"].as_str().unwrap();
    assert!(toml.contains("model_context_window = 16384"), "{toml}");

    let key = new_key(&e, "ctx").await;
    let req = Request::builder()
        .uri("/v1/models")
        .header(header::HOST, "enginex:8090")
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .body(Body::empty())
        .unwrap();
    let res = e.router.clone().oneshot(req).await.unwrap();
    let body: Value = serde_json::from_slice(&res.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(body["data"][0]["context_window"], 16384);

    // Flux chat/completions relayé tel quel : débit relevé au passage.
    assert!(e.get("/api/engine").await.body["last_gen"].is_null());
    let req = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header(header::HOST, "enginex:8090")
        .header(header::AUTHORIZATION, format!("Bearer {key}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({"model": "x", "stream": true, "stream_options": {"include_usage": true},
                "messages": [{"role": "user", "content": "salut"}]})
            .to_string(),
        ))
        .unwrap();
    let res = e.router.clone().oneshot(req).await.unwrap();
    let _ = res.into_body().collect().await.unwrap();
    let eng = e.until("/api/engine", |v| v["last_gen"].is_object()).await;
    assert_eq!(eng["last_gen"]["tokens_per_s"], 50.0);
}

/// Le contexte MTP ne se crée pas : relance seule sans décodage spéculatif,
/// signalée, puis retenue pour les lancements suivants.
#[tokio::test]
async fn spec_failure_falls_back() {
    let e = env_with("FAKE_SPEC_FAIL=1").await;
    let mtp = download(&e, MTP).await;
    let id = mtp["preset"]["id"].as_i64().unwrap();
    for round in 0..2 {
        let r = e
            .send(
                "POST",
                &format!("/api/presets/{id}/activate"),
                json!({"confirm_nofit": true}),
            )
            .await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.text);
        let want = if round == 0 {
            "relance sans"
        } else {
            "lancé sans décodage spéculatif"
        };
        e.until("/api/engine/logs", |v| {
            let lines = v["lines"].as_array().unwrap();
            lines.iter().filter_map(Value::as_str).any(|l| l.contains(want))
                && lines
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|l| l.contains("[enginex] prêt"))
        })
        .await;
        let eng = e.until("/api/engine", |v| v["state"]["state"] == "ready").await;
        assert_eq!(eng["spec_fallback"]["preset_id"], id);
        let why = eng["spec_fallback"]["why"].as_str().unwrap();
        assert!(why.contains("mtp context"), "{why}");
    }
}

/// Preset enregistré avant l'ajout de l'accélération : l'API le rend complet
/// (champs par défaut), sinon l'éditeur de l'UI plante.
#[tokio::test]
async fn old_presets_get_defaults() {
    let e = env().await;
    let d = download(&e, REPO).await;
    let id = d["preset"]["id"].as_i64().unwrap();
    sqlx::query("UPDATE presets SET settings = ? WHERE id = ?")
        .bind(r#"{"ctx": 8192, "kv_type": "q8_0"}"#)
        .bind(id)
        .execute(e.state.db())
        .await
        .unwrap();
    let ps = e.get("/api/presets").await.body;
    let s = &ps.as_array().unwrap().iter().find(|p| p["id"] == id).unwrap()["settings"];
    assert_eq!(s["ctx"], 8192);
    assert_eq!(s["spec"]["draft"], "none");
    assert_eq!(s["moe"]["mode"], "auto");
    assert!(s["batch"].is_null());
}

/// Brouillon refusé sans arrêt (b12464 continue sans) : signalé quand même.
#[tokio::test]
async fn ignored_draft_is_reported() {
    let e = env_with("FAKE_SPEC_FAIL=soft").await;
    let mtp = download(&e, MTP).await;
    let id = mtp["preset"]["id"].as_i64().unwrap();
    let r = e.send("POST", &format!("/api/presets/{id}/activate"), json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let eng = e
        .until("/api/engine", |v| {
            v["state"]["state"] == "ready" && v["spec_fallback"].is_object()
        })
        .await;
    assert!(eng["spec_fallback"]["why"].as_str().unwrap().contains("vocab"), "{eng}");
}
