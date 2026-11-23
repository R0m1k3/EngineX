use std::path::Path;

use enginex_engine::args::{
    BUDGET_MESSAGE, Caps, DraftKind, Launch, Moe, MoeMode, PresetSettings, Reasoning, Speculative, build_argv,
    fallback_argv, launch_env, needs_reload,
};
use enginex_hub::sampling::Sampling;

fn launch<'a>(model: &'a Path, mmproj: Option<&'a Path>, port: u16, caps: Caps) -> Launch<'a> {
    Launch {
        model,
        mmproj,
        draft: None,
        has_mtp: false,
        port,
        caps,
        extra_margin_mib: 0,
        mmproj_margin_mib: 0,
        gpus: 2,
    }
}

fn has(a: &[String], seq: &[&str]) -> bool {
    a.windows(seq.len()).any(|w| w.iter().zip(seq).all(|(x, y)| x == y))
}

#[test]
fn default_argv_uses_fit_and_no_sampling() {
    let p = PresetSettings {
        sampling: Sampling {
            temperature: Some(0.6),
            ..Default::default()
        },
        ..Default::default()
    };
    let a = build_argv(
        &p,
        &launch(
            Path::new("/models/m.gguf"),
            Some(Path::new("/models/mm.gguf")),
            8081,
            Caps::ALL,
        ),
    );
    assert!(has(
        &a,
        &[
            "-m",
            "/models/m.gguf",
            "--host",
            "127.0.0.1",
            "--port",
            "8081",
            "-c",
            "65536"
        ]
    ));
    assert!(has(&a, &["-ctk", "q8_0", "-ctv", "q8_0", "-sm", "layer"]));
    assert!(has(&a, &["--fit", "on", "--fit-target", "1024"]));
    assert!(has(&a, &["--mmproj", "/models/mm.gguf"]));
    assert!(!a.iter().any(|x| x.starts_with("--temp") || x == "-ngl" || x == "-mg"));
    assert!(!a.iter().any(|x| x.starts_with("--spec") || x.contains("moe")));
}

#[test]
fn mmproj_margin_only_on_first_card() {
    let p = PresetSettings::default();
    let mut l = launch(
        Path::new("/models/m.gguf"),
        Some(Path::new("/models/mm.gguf")),
        8081,
        Caps::ALL,
    );
    l.mmproj_margin_mib = 1900;
    assert!(has(&build_argv(&p, &l), &["--fit-target", "2924,1024"]));
    l.extra_margin_mib = 100;
    assert!(has(&build_argv(&p, &l), &["--fit-target", "3024,1124"]));
    l.gpus = 1;
    assert!(has(&build_argv(&p, &l), &["--fit-target", "3024"]));
}

#[test]
fn manual_split_mmproj_off_reasoning_off_extra_last() {
    let p = PresetSettings {
        tensor_split: Some(vec![16.0, 12.5]),
        mmproj: false,
        reasoning: Reasoning::Off,
        extra_args: vec!["-c".into(), "8192".into()],
        ..Default::default()
    };
    let a = build_argv(&p, &launch(Path::new("m"), Some(Path::new("mm")), 1, Caps::ALL));
    assert!(has(&a, &["-ngl", "999", "-ts", "16,12.5", "--fit", "off"]));
    assert!(!a.contains(&"--mmproj".to_string()));
    assert!(has(&a, &["--reasoning-budget", "0"]));
    assert_eq!(&a[a.len() - 2..], ["-c", "8192"]);
}

#[test]
fn image_without_fit_omits_fit_options() {
    let a = build_argv(&PresetSettings::default(), &launch(Path::new("m"), None, 1, Caps::NONE));
    assert!(!a.iter().any(|x| x.starts_with("--fit")));
}

#[test]
fn mtp_ngram_and_draft_model() {
    let mut p = PresetSettings {
        spec: Speculative {
            draft: DraftKind::Mtp,
            draft_max: 4,
            ngram: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut l = launch(Path::new("m"), None, 1, Caps::ALL);
    l.has_mtp = true;
    let a = build_argv(&p, &l);
    assert!(has(
        &a,
        &["--spec-type", "draft-mtp,ngram-mod", "--spec-draft-n-max", "4"]
    ));
    assert!(has(
        &a,
        &["--spec-ngram-mod-n-match", "24", "--spec-ngram-mod-n-min", "48"]
    ));
    // Repli : la même ligne sans aucune option spéculative.
    let fb = fallback_argv(&p, &l).unwrap();
    assert!(!fb.iter().any(|x| x.starts_with("--spec")));
    assert!(has(&fb, &["--fit", "on"]));
    // Experts en RAM : ngram retiré (vérification sur CPU), MTP gardé.
    p.moe = Moe {
        mode: MoeMode::AllCpu,
        ..Default::default()
    };
    let a = build_argv(&p, &l);
    assert!(has(&a, &["--spec-type", "draft-mtp", "--spec-draft-n-max", "4"]));
    assert!(!a.iter().any(|x| x.contains("ngram")));
    p.moe = Moe::default();
    // MTP demandé mais absent du GGUF : seul ngram reste, sans n-max.
    l.has_mtp = false;
    let a = build_argv(&p, &l);
    assert!(has(&a, &["--spec-type", "ngram-mod"]));
    assert!(!a.contains(&"--spec-draft-n-max".to_string()));
    // Petit modèle brouillon, marge VRAM ajoutée.
    p.spec.draft = DraftKind::Model;
    p.spec.draft_model_id = Some(7);
    p.spec.ngram = false;
    l.draft = Some(Path::new("/models/d.gguf"));
    l.extra_margin_mib = 600;
    let a = build_argv(&p, &l);
    assert!(has(&a, &["-md", "/models/d.gguf", "--spec-type", "draft-simple"]));
    assert!(has(&a, &["--fit", "on", "--fit-target", "1624"]));
    // Moteur sans ces options (retour arrière) : rien.
    l.caps = Caps {
        spec: false,
        ..Caps::ALL
    };
    assert!(!build_argv(&p, &l).iter().any(|x| x.starts_with("--spec") || x == "-md"));
}

#[test]
fn moe_manual_turns_fit_off() {
    let mut p = PresetSettings {
        moe: Moe {
            mode: MoeMode::Layers,
            cpu_layers: 12,
            cache_mib: 2048,
        },
        ..Default::default()
    };
    let a = build_argv(&p, &launch(Path::new("m"), None, 1, Caps::ALL));
    assert!(has(&a, &["-ngl", "999", "--fit", "off"]));
    assert!(has(&a, &["--n-cpu-moe", "12", "--moe-cache-mib", "2048"]));
    p.moe.mode = MoeMode::AllCpu;
    p.moe.cache_mib = 0;
    let a = build_argv(&p, &launch(Path::new("m"), None, 1, Caps::ALL));
    assert!(a.contains(&"--cpu-moe".to_string()));
    assert!(!a.iter().any(|x| x == "--moe-cache-mib" || x == "--n-cpu-moe"));
    // Auto : --fit garde la main.
    p.moe.mode = MoeMode::Auto;
    let a = build_argv(&p, &launch(Path::new("m"), None, 1, Caps::ALL));
    assert!(has(&a, &["--fit", "on"]));
    assert!(!a.iter().any(|x| x.contains("moe")));
}

#[test]
fn reload_rules() {
    let a = PresetSettings::default();
    let mut b = a.clone();
    b.sampling.top_k = Some(20.0);
    assert!(!needs_reload(&a, &b));
    b.ctx = 32768;
    assert!(needs_reload(&a, &b));
}

#[test]
fn validation_and_defaults_from_partial_json() {
    let p: PresetSettings = serde_json::from_str(r#"{"ctx": 32768}"#).unwrap();
    assert_eq!((p.ctx, p.kv_type.as_str(), p.parallel), (32768, "q8_0", 1));
    assert_eq!(p.invalid(), None);
    assert_eq!(
        PresetSettings {
            kv_type: "q5".into(),
            ..Default::default()
        }
        .invalid(),
        Some("kv_type")
    );
    assert_eq!(
        PresetSettings {
            ctx: 10,
            ..Default::default()
        }
        .invalid(),
        Some("ctx")
    );
}

#[test]
fn acceleration_validation() {
    let ok = PresetSettings::default();
    assert_eq!(ok.invalid(), None);
    let mut p = ok.clone();
    p.spec.draft_max = 0;
    assert_eq!(p.invalid(), Some("draft_max"));
    let mut p = ok.clone();
    p.spec.draft = DraftKind::Model;
    assert_eq!(p.invalid(), Some("draft_model_id"));
    let mut p = ok.clone();
    p.moe.mode = MoeMode::Layers;
    assert_eq!(p.invalid(), Some("cpu_layers"));
    // Anciens presets : champs absents → valeurs par défaut.
    let old: PresetSettings = serde_json::from_str(r#"{"ctx": 8192, "kv_type": "f16"}"#).unwrap();
    assert_eq!(
        (old.spec.draft, old.spec.draft_max, old.moe.mode),
        (DraftKind::None, 3, MoeMode::Auto)
    );
}

#[test]
fn batches_and_launch_queues() {
    let mut p = PresetSettings {
        batch: Some(4096),
        ubatch: Some(2048),
        ..Default::default()
    };
    let l = launch(Path::new("m"), None, 1, Caps::ALL);
    assert!(has(&build_argv(&p, &l), &["-b", "4096", "-ub", "2048"]));
    assert_eq!(p.invalid(), None);
    p.ubatch = Some(8192);
    assert_eq!(p.invalid(), Some("ubatch"));
    p.ubatch = Some(16);
    assert_eq!(p.invalid(), Some("ubatch"));
    p.ubatch = None;
    assert!(fallback_argv(&p, &l).is_none(), "sans spéculatif, pas de repli");

    // Files de lancement CUDA : 2 GPU en découpe par couches seulement.
    let q = vec![("CUDA_SCALE_LAUNCH_QUEUES".to_string(), "4x".to_string())];
    if std::env::var_os("CUDA_SCALE_LAUNCH_QUEUES").is_none() {
        assert_eq!(launch_env(&p, &l, 2), q);
    }
    assert!(launch_env(&p, &l, 1).is_empty());
    p.moe = Moe {
        mode: MoeMode::Layers,
        cpu_layers: 4,
        cache_mib: 0,
    };
    assert!(launch_env(&p, &l, 2).is_empty());
    p.moe = Moe::default();
    p.extra_args = vec!["-ot".into(), "exps=CPU".into()];
    assert!(launch_env(&p, &l, 2).is_empty());
}

#[test]
fn reasoning_budget_and_max_output() {
    let p = PresetSettings::default();
    let m = Path::new("/models/m.gguf");
    let mut l = launch(m, None, 8081, Caps::ALL);
    let a = build_argv(&p, &l);
    assert!(has(
        &a,
        &[
            "--reasoning-budget",
            "16384",
            "--reasoning-budget-message",
            BUDGET_MESSAGE
        ]
    ));
    assert!(has(&a, &["-n", "32768"]));
    // Réflexion coupée : budget 0, sans message.
    let off = PresetSettings {
        reasoning: Reasoning::Off,
        ..p.clone()
    };
    let a = build_argv(&off, &l);
    assert!(has(&a, &["--reasoning-budget", "0"]) && !a.iter().any(|x| x == "--reasoning-budget-message"));
    // Illimité, ou moteur sans l'option : rien.
    let free = PresetSettings {
        reasoning_budget: None,
        max_output: None,
        ..p.clone()
    };
    let a = build_argv(&free, &l);
    assert!(!a.iter().any(|x| x == "--reasoning-budget" || x == "-n"));
    l.caps = Caps {
        budget: false,
        ..Caps::ALL
    };
    assert!(!build_argv(&p, &l).iter().any(|x| x == "--reasoning-budget"));
    // Bornes.
    assert_eq!(
        PresetSettings {
            reasoning_budget: Some(10),
            ..p.clone()
        }
        .invalid(),
        Some("reasoning_budget")
    );
    assert_eq!(
        PresetSettings {
            max_output: Some(10),
            ..p
        }
        .invalid(),
        Some("max_output")
    );
}
