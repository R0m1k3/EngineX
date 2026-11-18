use std::path::Path;

use enginex_engine::args::{PresetSettings, Reasoning, build_argv, needs_reload};
use enginex_hub::sampling::Sampling;

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
        Path::new("/models/m.gguf"),
        Some(Path::new("/models/mm.gguf")),
        8081,
        true,
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
    let a = build_argv(&p, Path::new("m"), Some(Path::new("mm")), 1, true);
    assert!(has(&a, &["-ngl", "999", "-ts", "16,12.5", "--fit", "off"]));
    assert!(!a.contains(&"--mmproj".to_string()));
    assert!(has(&a, &["--reasoning-budget", "0"]));
    assert_eq!(&a[a.len() - 2..], ["-c", "8192"]);
}

#[test]
fn image_without_fit_omits_fit_options() {
    let a = build_argv(&PresetSettings::default(), Path::new("m"), None, 1, false);
    assert!(!a.iter().any(|x| x.starts_with("--fit")));
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
