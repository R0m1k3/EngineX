//! Échantillonnage du preset injecté dans chaque requête : une valeur du
//! preset remplace celle du client ; absente, celle du client passe.

use enginex_hub::sampling::Sampling;
use serde_json::{Value, json};

pub fn inject(body: &mut Value, s: &Sampling) {
    let Some(obj) = body.as_object_mut() else { return };
    let pairs = [
        ("temperature", s.temperature),
        ("top_p", s.top_p),
        ("min_p", s.min_p),
        ("presence_penalty", s.presence_penalty),
        ("repeat_penalty", s.repeat_penalty),
    ];
    for (k, v) in pairs {
        if let Some(v) = v {
            obj.insert(k.into(), json!(v));
        }
    }
    if let Some(k) = s.top_k {
        // llama-server lit un entier.
        let v = if k.fract() == 0.0 { json!(k as i64) } else { json!(k) };
        obj.insert("top_k".into(), v);
    }
}

/// Valeurs appliquées, pour le journal (`temperature=0.6 top_k=20`).
pub fn describe(body: &Value) -> String {
    [
        "temperature",
        "top_p",
        "top_k",
        "min_p",
        "presence_penalty",
        "repeat_penalty",
    ]
    .iter()
    .filter_map(|k| body.get(*k).map(|v| format!("{k}={v}")))
    .collect::<Vec<_>>()
    .join(" ")
}
