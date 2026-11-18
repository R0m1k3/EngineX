//! Coffre : chiffrement, coffre verrouillé, noms, valeurs jamais rendues.

mod common;

use std::os::unix::fs::PermissionsExt;

use axum::http::StatusCode;
use common::env;
use enginex_server::vault::{Vault, decrypt, encrypt, valid_name};
use serde_json::json;

#[test]
fn encryption_is_bound_to_scope_and_name() {
    let key = [7u8; 32];
    let (nonce, ct) = encrypt(&key, "global", "GITHUB_TOKEN", b"valeur");
    assert_eq!(decrypt(&key, "global", "GITHUB_TOKEN", &nonce, &ct).unwrap(), b"valeur");
    assert!(decrypt(&key, "global", "OTHER", &nonce, &ct).is_none());
    assert!(decrypt(&key, "project:1", "GITHUB_TOKEN", &nonce, &ct).is_none());
    assert!(decrypt(&[8u8; 32], "global", "GITHUB_TOKEN", &nonce, &ct).is_none());
}

#[test]
fn names() {
    for ok in ["GITHUB_TOKEN", "A", "_X1"] {
        assert!(valid_name(ok), "{ok}");
    }
    for bad in [
        "github",
        "1ABC",
        "A-B",
        "ENGINEX_KEY",
        "CODEX_HOME",
        "PATH",
        "HOME",
        "USER",
        "",
    ] {
        assert!(!valid_name(bad), "{bad}");
    }
    assert!(!valid_name(&"A".repeat(65)));
}

#[tokio::test]
async fn key_file_values_and_lock() {
    let e = env().await;
    let key = e.dir.path().join("data/master.key");
    let meta = std::fs::metadata(&key).unwrap();
    assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    assert_eq!(meta.len(), 32);

    let mut bodies = Vec::new();
    let r = e
        .send(
            "PUT",
            "/api/vault",
            json!({"scope": "global", "name": "GITHUB_TOKEN", "value": "ghp_tres_secret"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    bodies.push(r.text);
    let r = e
        .send(
            "PUT",
            "/api/vault",
            json!({"scope": "global", "name": "bad-name", "value": "x"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.body["error"], "invalid_name");
    let r = e
        .send(
            "PUT",
            "/api/vault",
            json!({"scope": "project:999", "name": "X", "value": "x"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    let r = e.get("/api/vault").await;
    assert_eq!(r.body["locked"], false);
    assert_eq!(r.body["secrets"][0]["name"], "GITHUB_TOKEN");
    bodies.push(r.text);
    for path in [
        "/api/projects",
        "/api/api-keys",
        "/api/sandbox/status",
        "/api/settings/sessions",
    ] {
        bodies.push(e.get(path).await.text);
    }
    for b in &bodies {
        assert!(!b.contains("ghp_tres_secret"), "{b}");
    }

    // Clé maîtresse perdue : coffre verrouillé, aucune clé recréée.
    std::fs::remove_file(&key).unwrap();
    Vault::open(&e.state).await.unwrap();
    assert!(!key.exists());
    assert!(e.state.vault_locked());
    let r = e.get("/api/vault").await;
    assert_eq!(r.body["locked"], true);
    let r = e
        .send(
            "PUT",
            "/api/vault",
            json!({"scope": "global", "name": "OTHER", "value": "x"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    assert_eq!(r.body["error"], "vault_locked");
    // Les secrets supprimés, une nouvelle clé est créée.
    let id = e.get("/api/vault").await.body["secrets"][0]["id"].as_i64().unwrap();
    let r = e.send("DELETE", &format!("/api/vault/{id}"), json!({})).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(key.exists());
    assert!(!e.state.vault_locked());
}
