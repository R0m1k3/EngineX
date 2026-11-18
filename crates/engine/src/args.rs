//! Réglages d'un preset et ligne de commande de llama-server.
//!
//! L'échantillonnage n'est jamais dans la ligne de commande : il est injecté
//! dans chaque requête par EngineX (étape 3), ce qui permet de le modifier sans
//! recharger le modèle.

use std::path::Path;

use enginex_hub::sampling::Sampling;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reasoning {
    #[default]
    On,
    Off,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PresetSettings {
    pub ctx: u64,
    pub kv_type: String,
    pub flash_attn: bool,
    pub parallel: u64,
    pub fit_target_mib: u64,
    pub tensor_split: Option<Vec<f64>>,
    pub reasoning: Reasoning,
    pub sampling: Sampling,
    pub mmproj: bool,
    pub extra_args: Vec<String>,
}

impl Default for PresetSettings {
    fn default() -> Self {
        Self {
            ctx: 65536,
            kv_type: "q8_0".into(),
            flash_attn: true,
            parallel: 1,
            fit_target_mib: 1024,
            tensor_split: None,
            reasoning: Reasoning::On,
            sampling: Sampling::default(),
            mmproj: true,
            extra_args: Vec::new(),
        }
    }
}

impl PresetSettings {
    /// Valeurs refusées (message pour l'UI), `None` si tout va bien.
    pub fn invalid(&self) -> Option<&'static str> {
        if !(512..=1_048_576).contains(&self.ctx) {
            return Some("ctx");
        }
        if !matches!(self.kv_type.as_str(), "f16" | "q8_0" | "q4_0") {
            return Some("kv_type");
        }
        if !(1..=8).contains(&self.parallel) {
            return Some("parallel");
        }
        if self.fit_target_mib > 16384 {
            return Some("fit_target_mib");
        }
        if let Some(ts) = &self.tensor_split
            && (ts.is_empty() || ts.iter().any(|v| !v.is_finite() || *v < 0.0))
        {
            return Some("tensor_split");
        }
        None
    }
}

fn num(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// Ligne de commande (sans le binaire). llama-server garde la dernière
/// occurrence d'une option : `extra_args` vient donc en dernier.
pub fn build_argv(p: &PresetSettings, model: &Path, mmproj: Option<&Path>, port: u16, has_fit: bool) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "-m".into(),
        model.display().to_string(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        port.to_string(),
        "-c".into(),
        p.ctx.to_string(),
        "-np".into(),
        p.parallel.to_string(),
        "--jinja".into(),
        "-fa".into(),
        if p.flash_attn { "on" } else { "off" }.into(),
        "-ctk".into(),
        p.kv_type.clone(),
        "-ctv".into(),
        p.kv_type.clone(),
        "-sm".into(),
        "layer".into(),
    ];
    match &p.tensor_split {
        Some(ts) => {
            a.extend(["-ngl".into(), "999".into(), "-ts".into()]);
            a.push(ts.iter().map(|v| num(*v)).collect::<Vec<_>>().join(","));
            if has_fit {
                a.extend(["--fit".into(), "off".into()]);
            }
        }
        None if has_fit => {
            a.extend([
                "--fit".into(),
                "on".into(),
                "--fit-target".into(),
                p.fit_target_mib.to_string(),
            ]);
        }
        None => {}
    }
    if let (true, Some(mm)) = (p.mmproj, mmproj) {
        a.extend(["--mmproj".into(), mm.display().to_string()]);
    }
    if p.reasoning == Reasoning::Off {
        a.extend(["--reasoning-budget".into(), "0".into()]);
    }
    a.extend(p.extra_args.iter().cloned());
    a
}

/// Faut-il recharger le modèle pour passer de `old` à `new` ? Non si seul
/// l'échantillonnage change.
pub fn needs_reload(old: &PresetSettings, new: &PresetSettings) -> bool {
    let strip = |p: &PresetSettings| PresetSettings {
        sampling: Sampling::default(),
        ..p.clone()
    };
    strip(old) != strip(new)
}
