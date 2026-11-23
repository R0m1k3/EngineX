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

/// Source du brouillon pour le décodage spéculatif.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftKind {
    #[default]
    None,
    /// Couches MTP (NextN) du modèle lui-même (`--spec-type draft-mtp`).
    Mtp,
    /// Petit modèle local de la même famille (`-md`, `draft-simple`).
    Model,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Speculative {
    pub draft: DraftKind,
    /// Modèle local servant de brouillon (`draft = model`).
    pub draft_model_id: Option<i64>,
    /// Jetons proposés par étape (`--spec-draft-n-max`).
    pub draft_max: u64,
    /// Brouillon tiré des répétitions du contexte (`ngram-mod`), sans VRAM.
    pub ngram: bool,
}

impl Default for Speculative {
    fn default() -> Self {
        Self {
            draft: DraftKind::None,
            draft_model_id: None,
            draft_max: 3,
            ngram: false,
        }
    }
}

/// Place des poids des experts d'un modèle MoE.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MoeMode {
    /// `--fit` décide (experts des couches en trop envoyés en RAM).
    #[default]
    Auto,
    /// Tous les experts en RAM (`--cpu-moe`), le reste sur les GPU.
    AllCpu,
    /// Experts des `cpu_layers` premières couches en RAM (`--n-cpu-moe`).
    Layers,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Moe {
    pub mode: MoeMode,
    pub cpu_layers: u64,
    /// Cache GPU des experts gardés en RAM (`--moe-cache-mib`), 0 = sans.
    pub cache_mib: u64,
}

impl Moe {
    /// Placement manuel : `--fit` ne s'applique plus (il refuse des
    /// `tensor_buft_overrides` posés par l'utilisateur).
    pub fn manual(&self) -> bool {
        self.mode != MoeMode::Auto
    }
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
    pub spec: Speculative,
    pub moe: Moe,
    /// Lots logique et physique (`-b`, `-ub`) ; `None` = défaut de llama.cpp
    /// (2048 / 512). Un MoE plus gros que la VRAM gagne à 4096 / 2048.
    pub batch: Option<u64>,
    pub ubatch: Option<u64>,
    /// Jetons de réflexion au plus par réponse (`--reasoning-budget`) ; `None`
    /// = illimité. Au-delà, llama.cpp ferme la réflexion et le modèle répond :
    /// évite les réflexions qui tournent jusqu'à remplir le contexte.
    pub reasoning_budget: Option<u64>,
    /// Jetons écrits au plus par réponse (`-n`), réflexion comprise ; `None`
    /// = jusqu'à la fin du contexte.
    pub max_output: Option<u64>,
    pub extra_args: Vec<String>,
}

/// Réflexion par défaut : large pour du code, mais bornée.
pub const REASONING_BUDGET: u64 = 16384;
pub const MAX_OUTPUT: u64 = 32768;
/// Injecté avant la fin de la réflexion quand le budget est épuisé.
pub const BUDGET_MESSAGE: &str = "\n\nI have thought about this long enough. Time to stop thinking and act on it.\n";

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
            spec: Speculative::default(),
            moe: Moe::default(),
            batch: None,
            ubatch: None,
            reasoning_budget: Some(REASONING_BUDGET),
            max_output: Some(MAX_OUTPUT),
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
        if !(1..=16).contains(&self.spec.draft_max) {
            return Some("draft_max");
        }
        if (self.spec.draft == DraftKind::Model) != self.spec.draft_model_id.is_some() {
            return Some("draft_model_id");
        }
        if self.moe.cpu_layers > 1024 || (self.moe.mode == MoeMode::Layers && self.moe.cpu_layers == 0) {
            return Some("cpu_layers");
        }
        if self.moe.cache_mib > 65536 {
            return Some("cache_mib");
        }
        let lot = |v: Option<u64>| v.is_some_and(|v| !(32..=16384).contains(&v));
        if lot(self.batch) {
            return Some("batch");
        }
        if lot(self.ubatch) || matches!((self.batch, self.ubatch), (Some(b), Some(u)) if u > b) {
            return Some("ubatch");
        }
        if self.reasoning_budget.is_some_and(|v| !(256..=1_048_576).contains(&v)) {
            return Some("reasoning_budget");
        }
        if self.max_output.is_some_and(|v| !(256..=1_048_576).contains(&v)) {
            return Some("max_output");
        }
        None
    }
}

/// Options que le binaire llama-server connaît (lues dans son `--help`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps {
    pub fit: bool,
    pub spec: bool,
    pub moe: bool,
    /// Budget de réflexion en jetons avec message (`--reasoning-budget-message`).
    pub budget: bool,
}

impl Caps {
    pub const ALL: Caps = Caps {
        fit: true,
        spec: true,
        moe: true,
        budget: true,
    };
    pub const NONE: Caps = Caps {
        fit: false,
        spec: false,
        moe: false,
        budget: false,
    };
}

/// Fichiers et contexte de lancement d'un preset.
#[derive(Clone, Copy)]
pub struct Launch<'a> {
    pub model: &'a Path,
    pub mmproj: Option<&'a Path>,
    /// GGUF du petit modèle brouillon (`draft = model`).
    pub draft: Option<&'a Path>,
    /// Le GGUF contient des couches MTP.
    pub has_mtp: bool,
    pub port: u16,
    pub caps: Caps,
    /// Marge VRAM ajoutée à `fit_target_mib` (brouillon, que `--fit` ne
    /// compte pas).
    pub extra_margin_mib: u64,
    /// Marge VRAM ajoutée sur la première carte seulement : le projecteur de
    /// vision (mmproj), chargé après le modèle sur CUDA0, que `--fit` ne
    /// compte pas non plus.
    pub mmproj_margin_mib: u64,
    /// Nombre de cartes (pour la marge par carte de `--fit-target`).
    pub gpus: usize,
}

/// Valeur de `--fit-target` : une marge commune, ou une liste par carte
/// quand la première porte en plus le mmproj.
fn fit_target(base: u64, l: &Launch) -> String {
    let all = base + l.extra_margin_mib;
    if l.mmproj_margin_mib == 0 {
        return all.to_string();
    }
    let first = all + l.mmproj_margin_mib;
    std::iter::once(first)
        .chain(std::iter::repeat_n(all, l.gpus.saturating_sub(1)))
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(",")
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
pub fn build_argv(p: &PresetSettings, l: &Launch) -> Vec<String> {
    let mut a: Vec<String> = vec![
        "-m".into(),
        l.model.display().to_string(),
        "--host".into(),
        "127.0.0.1".into(),
        "--port".into(),
        l.port.to_string(),
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
    let moe_manual = l.caps.moe && p.moe.manual();
    if p.tensor_split.is_some() || moe_manual {
        a.extend(["-ngl".into(), "999".into()]);
        if let Some(ts) = &p.tensor_split {
            a.push("-ts".into());
            a.push(ts.iter().map(|v| num(*v)).collect::<Vec<_>>().join(","));
        }
        if l.caps.fit {
            a.extend(["--fit".into(), "off".into()]);
        }
    } else if l.caps.fit {
        a.extend([
            "--fit".into(),
            "on".into(),
            "--fit-target".into(),
            fit_target(p.fit_target_mib, l),
        ]);
    }
    if moe_manual {
        match p.moe.mode {
            MoeMode::AllCpu => a.push("--cpu-moe".into()),
            MoeMode::Layers => a.extend(["--n-cpu-moe".into(), p.moe.cpu_layers.to_string()]),
            MoeMode::Auto => {}
        }
        if p.moe.cache_mib > 0 {
            a.extend(["--moe-cache-mib".into(), p.moe.cache_mib.to_string()]);
        }
    }
    if let Some(b) = p.batch {
        a.extend(["-b".into(), b.to_string()]);
    }
    if let Some(u) = p.ubatch {
        a.extend(["-ub".into(), u.to_string()]);
    }
    if let (true, Some(mm)) = (p.mmproj, l.mmproj) {
        a.extend(["--mmproj".into(), mm.display().to_string()]);
    }
    if l.caps.spec {
        let mut types = Vec::new();
        match (p.spec.draft, l.draft) {
            (DraftKind::Mtp, _) if l.has_mtp => types.push("draft-mtp"),
            (DraftKind::Model, Some(d)) => {
                types.push("draft-simple");
                a.extend(["-md".into(), d.display().to_string()]);
            }
            _ => {}
        }
        let drafting = !types.is_empty();
        // ngram propose des lots de quelques jetons : avec des experts en RAM,
        // la vérification passe par le CPU (sous GGML_OP_OFFLOAD_MIN_BATCH) et
        // ralentit au lieu d'accélérer (règle de Loki).
        let ngram = p.spec.ngram && !moe_manual;
        if ngram {
            types.push("ngram-mod");
        }
        if !types.is_empty() {
            a.extend(["--spec-type".into(), types.join(",")]);
        }
        if drafting {
            a.extend(["--spec-draft-n-max".into(), p.spec.draft_max.to_string()]);
        }
        if ngram {
            // Valeurs de `--spec-default` (b12464), reprises par Loki.
            for (k, v) in [
                ("--spec-ngram-mod-n-match", "24"),
                ("--spec-ngram-mod-n-min", "48"),
                ("--spec-ngram-mod-n-max", "64"),
            ] {
                a.extend([k.into(), v.into()]);
            }
        }
    }
    match (p.reasoning, p.reasoning_budget) {
        (Reasoning::Off, _) => a.extend(["--reasoning-budget".into(), "0".into()]),
        (Reasoning::On, Some(n)) if l.caps.budget => a.extend([
            "--reasoning-budget".into(),
            n.to_string(),
            "--reasoning-budget-message".into(),
            BUDGET_MESSAGE.into(),
        ]),
        (Reasoning::On, _) => {}
    }
    if let Some(n) = p.max_output {
        a.extend(["-n".into(), n.to_string()]);
    }
    a.extend(p.extra_args.iter().cloned());
    a
}

/// Le même lancement sans décodage spéculatif (repli si le chargement
/// échoue avec), `None` s'il n'y en avait pas.
pub fn fallback_argv(p: &PresetSettings, l: &Launch) -> Option<Vec<String>> {
    let with = build_argv(p, l);
    if !with.iter().any(|a| a == "--spec-type") {
        return None;
    }
    let caps = Caps { spec: false, ..l.caps };
    let mut no = Launch { caps, ..*l };
    no.extra_margin_mib = 0;
    Some(build_argv(p, &no))
}

/// Environnement de llama-server : `CUDA_SCALE_LAUNCH_QUEUES=4x` dès 2 GPU
/// en découpe par couches sans tenseurs placés à la main (parallélisme de
/// pipeline, règle de Loki).
pub fn launch_env(p: &PresetSettings, l: &Launch, n_gpus: usize) -> Vec<(String, String)> {
    const BLOCKERS: [&str; 10] = [
        "-ot",
        "--override-tensor",
        "-sm",
        "--split-mode",
        "-ngl",
        "--n-gpu-layers",
        "-nkvo",
        "--no-kv-offload",
        "--cpu-moe",
        "--n-cpu-moe",
    ];
    let manual = l.caps.moe && p.moe.manual();
    let blocked = p.extra_args.iter().any(|a| BLOCKERS.contains(&a.as_str()));
    if n_gpus < 2 || manual || blocked || std::env::var_os("CUDA_SCALE_LAUNCH_QUEUES").is_some() {
        return Vec::new();
    }
    vec![("CUDA_SCALE_LAUNCH_QUEUES".into(), "4x".into())]
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
