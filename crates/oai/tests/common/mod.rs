#![allow(dead_code)]

use std::path::PathBuf;

use serde_json::Value;

pub fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

pub fn fixture(name: &str) -> Value {
    let raw = std::fs::read_to_string(fixture_path(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// Compare à un fichier attendu ; `UPDATE_FIXTURES=1` le réécrit.
pub fn golden(name: &str, got: &Value) {
    let path = fixture_path(name);
    if std::env::var("UPDATE_FIXTURES").as_deref() == Ok("1") {
        std::fs::write(&path, serde_json::to_string_pretty(got).unwrap() + "\n").unwrap();
        return;
    }
    let want = fixture(name);
    assert_eq!(got, &want, "{name} diffère (UPDATE_FIXTURES=1 pour réécrire)");
}

/// Identifiants par compteur : `rs_1`, `msg_2`…
pub fn counter_ids() -> Box<dyn FnMut(&str) -> String + Send> {
    let mut n = 0;
    Box::new(move |p: &str| {
        n += 1;
        format!("{p}_{n}")
    })
}
