//! Place estimée d'un preset en cours d'édition (`/api/presets/{id}/fit`),
//! avec deux fausses cartes (5060 Ti 16 Go + 3060 12 Go).

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;

#[tokio::test]
async fn fit_follows_edited_settings() {
    let gpus = tempfile::tempdir().unwrap();
    let smi = gpus.path().join("nvidia-smi");
    std::fs::write(
        &smi,
        "#!/bin/sh\necho '0, NVIDIA GeForce RTX 5060 Ti, 16311, 500, 40, 595.112.07'\n\
         echo '1, NVIDIA GeForce RTX 3060, 12288, 300, 45, 595.112.07'\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&smi, std::fs::Permissions::from_mode(0o755)).unwrap();
    let e = env_cfg("", |c| c.nvidia_smi = smi.clone()).await;
    let id = make_active_preset(&e).await;
    let p = e.get("/api/presets").await.body[0].clone();
    let mut s = p["settings"].clone();

    let r = e
        .send("POST", &format!("/api/presets/{id}/fit"), json!({"settings": s}))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.body["verdict"], "fits", "{}", r.body);
    assert_eq!(
        (r.body["no_fa_mib"].as_u64(), r.body["cpu_mib"].as_u64()),
        (Some(0), Some(0))
    );
    assert_eq!(r.body["cards"].as_array().unwrap().len(), 2);

    // Sans flash attention : 16 têtes × 512 × contexte × 4 octets par carte.
    s["flash_attn"] = json!(false);
    let ctx = s["ctx"].as_u64().unwrap();
    let r = e
        .send("POST", &format!("/api/presets/{id}/fit"), json!({"settings": s}))
        .await;
    assert_eq!(
        r.body["no_fa_mib"].as_u64(),
        Some((16 * 512 * ctx * 4).div_ceil(1 << 20)),
        "{}",
        r.body
    );

    // Réglages invalides ou preset absent.
    s["ctx"] = json!(10);
    let r = e
        .send("POST", &format!("/api/presets/{id}/fit"), json!({"settings": s}))
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = e
        .send("POST", "/api/presets/999/fit", json!({"settings": p["settings"]}))
        .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}
