//! Parcours complet de l'étape 2 avec le faux llama-server, un faux Hugging
//! Face et un faux registre : téléchargement, preset auto, activation,
//! installation d'une version, bascule, retour arrière et retour automatique.

mod common;

use axum::http::StatusCode;
use common::{REPO, env, publish};
use serde_json::{Value, json};

fn engine_state(v: &Value) -> &str {
    v["state"]["state"].as_str().unwrap_or("")
}

#[tokio::test]
async fn models_presets_engine_and_versions() {
    let e = env().await;

    // Recherche et fiche du dépôt.
    let r = e.get("/api/hf/search?q=qwen").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.body[0]["repo"], REPO);
    let r = e.get(&format!("/api/hf/repo?repo={REPO}")).await;
    assert_eq!(r.body["groups"][0]["quant"], "Q8_0", "{}", r.text);
    assert_eq!(e.get("/api/hf/repo?repo=../x").await.status, StatusCode::BAD_REQUEST);
    let r = e
        .get(&format!("/api/hf/fit?repo={REPO}&file=Qwen3-0.6B-Q8_0.gguf&ctx=32768"))
        .await;
    assert_eq!(r.body["report"]["verdict"], "unknown", "{}", r.text);
    assert_eq!(r.body["meta"]["block_count"], 28);

    // Téléchargement jusqu'au bout, puis doublon refusé.
    let r = e
        .send("POST", "/api/downloads", json!({"repo": REPO, "quant": "Q8_0"}))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    e.until("/api/downloads", |v| v[0]["state"] == "done").await;
    assert!(
        e.dir
            .path()
            .join("models")
            .join(REPO)
            .join("Qwen3-0.6B-Q8_0.gguf")
            .is_file()
    );
    let r = e
        .send("POST", "/api/downloads", json!({"repo": REPO, "quant": "Q8_0"}))
        .await;
    assert_eq!(
        (r.status, r.body["error"].as_str()),
        (StatusCode::CONFLICT, Some("already_downloaded"))
    );

    // Modèle local et preset auto avec ses sources.
    let models = e.get("/api/models").await.body;
    let model_id = models[0]["id"].as_i64().unwrap();
    assert_eq!(models[0]["meta"]["arch"], "qwen3");
    let presets = e.get("/api/presets").await.body;
    let p = &presets[0];
    let id = p["id"].as_i64().unwrap();
    assert_eq!(p["settings"]["ctx"], 40960, "contexte borné par le GGUF");
    assert!(p["settings"]["sampling"]["temperature"].is_number(), "{p}");
    let src = p["sources"]["origins"]["temperature"]["source"].as_str().unwrap();
    assert_ne!(src, "defaut", "{p}");

    // Activation → prêt, sondes faites.
    let r = e.send("POST", &format!("/api/presets/{id}/activate"), json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.body["started"], true, "{}", r.text);
    assert!(r.body["fit"].is_object(), "{}", r.text);
    let v = e
        .until("/api/engine/versions", |v| {
            v["job"]["kind"] == "activate" && v["job"]["finished"] == true
        })
        .await;
    assert_eq!(v["job"]["step"], "done", "{v}");
    let v = e
        .until("/api/engine", |v| engine_state(v) == "ready" && v["tools_ok"] == true)
        .await;
    assert_eq!(v["n_ctx"], 40960);
    assert_eq!(v["origin"], "image");
    assert_eq!(v["preset_id"], id);

    // Modifier l'échantillonnage seul ne recharge pas ; le contexte, si.
    let mut s = p["settings"].clone();
    s["sampling"]["temperature"] = json!(0.3);
    let r = e
        .send(
            "PUT",
            &format!("/api/presets/{id}"),
            json!({"name": "Qwen", "settings": s}),
        )
        .await;
    assert_eq!(r.body, json!({"reload_needed": false}), "{}", r.text);
    s["ctx"] = json!(8192);
    let r = e
        .send(
            "PUT",
            &format!("/api/presets/{id}"),
            json!({"name": "Qwen", "settings": s}),
        )
        .await;
    assert_eq!(r.body, json!({"reload_needed": true}));

    // Le modèle du preset actif ne se supprime pas.
    let r = e.send("DELETE", &format!("/api/models/{model_id}"), json!({})).await;
    assert_eq!(
        (r.status, r.body["error"].as_str()),
        (StatusCode::CONFLICT, Some("model_in_use"))
    );

    // Installation d'une version publiée → bascule et relance.
    publish(&e.reg, "b12500", "");
    let r = e.send("POST", "/api/engine/versions/check", json!({})).await;
    assert_eq!(r.body["tag"], "server-cuda-b12500", "{}", r.text);
    let r = e.send("POST", "/api/engine/versions/install", json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let v = e.until("/api/engine/versions", |v| v["job"]["finished"] == true).await;
    assert_eq!(v["job"]["step"], "done", "{v}");
    assert_eq!(v["active"], "server-cuda-b12500");
    assert_eq!(v["previous"], "image");
    let v = e.until("/api/engine", |v| engine_state(v) == "ready").await;
    assert_eq!(v["origin"], "active");
    assert_eq!(v["n_ctx"], 8192);
    let r = e
        .send("DELETE", "/api/engine/versions/server-cuda-b12500", json!({}))
        .await;
    assert_eq!(r.body["error"], "engine_in_use");

    // Retour à la précédente : les deux s'échangent.
    let r = e.send("POST", "/api/engine/versions/rollback", json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let v = e
        .until("/api/engine/versions", |v| {
            v["job"]["kind"] == "rollback" && v["job"]["finished"] == true
        })
        .await;
    assert_eq!(v["job"]["step"], "done", "{v}");
    assert_eq!(
        (v["active"].as_str(), v["previous"].as_str()),
        (Some("image"), Some("server-cuda-b12500"))
    );

    // Nouvelle version qui ne charge pas : retour automatique à l'ancienne.
    publish(&e.reg, "b12600", "FAKE_OOM=1");
    e.send("POST", "/api/engine/versions/check", json!({})).await;
    let r = e.send("POST", "/api/engine/versions/install", json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let v = e
        .until("/api/engine/versions", |v| {
            v["job"]["tag"] == "server-cuda-b12600" && v["job"]["finished"] == true
        })
        .await;
    assert_eq!(v["job"]["step"], "failed", "{v}");
    let err = v["job"]["error"].as_str().unwrap();
    assert!(err.starts_with("load_failed") && err.contains("VRAM"), "{err}");
    assert_eq!(
        (v["active"].as_str(), v["previous"].as_str()),
        (Some("image"), Some("server-cuda-b12500"))
    );
    let v = e.until("/api/engine", |v| engine_state(v) == "ready").await;
    assert_eq!(v["origin"], "image");

    // Arrêt (tâche en fond), puis suppression du modèle (et de ses presets).
    let r = e.send("POST", "/api/engine/stop", json!({})).await;
    assert_eq!(r.body["started"], true, "{}", r.text);
    let v = e
        .until("/api/engine/versions", |v| {
            v["job"]["kind"] == "stop" && v["job"]["finished"] == true
        })
        .await;
    assert_eq!(v["job"]["step"], "done", "{v}");
    let r = e.send("DELETE", &format!("/api/models/{model_id}"), json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(e.get("/api/presets").await.body, json!([]));
    assert!(!e.dir.path().join("models/unsloth").exists());
}

#[tokio::test]
async fn download_pause_cancel_and_tags() {
    let e = env().await;
    let r = e
        .send("POST", "/api/downloads", json!({"repo": REPO, "quant": "Q4_K_M"}))
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST, "quant absent du dépôt");
    let r = e.send("POST", "/api/engine/versions/nimporte/use", json!({})).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = e
        .send("POST", "/api/engine/versions/server-cuda-b1/use", json!({}))
        .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    let r = e.send("DELETE", "/api/engine/versions/image", json!({})).await;
    assert_eq!(r.body["error"], "engine_in_use");
    let r = e.send("POST", "/api/presets/99/activate", json!({})).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    // Sans session : refusé.
    let r = common::call(&e.router, "GET", "/api/engine", None, None).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
}
