//! Parseur de cards sur des cards réelles (enregistrées le 2026-11-18) et
//! fusion des sources.

use enginex_hub::sampling::{CandidateSet, Label, Sampling, SourceInputs, from_generation_config, merge, parse_card};

fn card(name: &str) -> String {
    std::fs::read_to_string(format!("{}/tests/cards/{name}.md", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn sets(name: &str, label: Label) -> Vec<CandidateSet> {
    parse_card(&card(name))
        .into_iter()
        .filter(|s| s.label == label)
        .collect()
}

fn s(t: f64, p: f64, k: f64, m: f64) -> Sampling {
    Sampling {
        temperature: Some(t),
        top_p: Some(p),
        top_k: Some(k),
        min_p: Some(m),
        ..Default::default()
    }
}

#[test]
fn qwen3_cards_have_both_modes() {
    for name in ["qwen3-0.6b", "qwen3-32b", "unsloth-qwen3-0.6b-gguf"] {
        let think = sets(name, Label::Thinking);
        let non = sets(name, Label::NonThinking);
        assert!(!think.is_empty() && !non.is_empty(), "{name}");
        assert!(
            think.iter().all(|x| x.values == s(0.6, 0.95, 20.0, 0.0)),
            "{name}: {think:?}"
        );
        assert!(
            non.iter().all(|x| x.values == s(0.7, 0.8, 20.0, 0.0)),
            "{name}: {non:?}"
        );
        // « presence_penalty entre 0 et 2 » n'est pas une valeur.
        assert!(
            parse_card(&card(name))
                .iter()
                .all(|x| x.values.presence_penalty.is_none())
        );
    }
}

#[test]
fn devstral_code_example() {
    let all = parse_card(&card("devstral-small"));
    assert!(
        all.iter()
            .any(|x| x.label == Label::General && x.values.temperature == Some(0.15)),
        "{all:?}"
    );
}

#[test]
fn cards_without_settings() {
    for name in ["sans-reglages", "gemma-3-27b-it", "glm-4-32b"] {
        assert!(
            parse_card(&card(name)).is_empty(),
            "{name}: {:?}",
            parse_card(&card(name))
        );
    }
}

#[test]
fn labels_order_and_forms() {
    let md = "## Recommended settings\n\n\
        - No think mode: temperature=0.7\n\
        - Reasoning: `temp: 0.6`, top-k = 40\n\
        - Default: --temp 0.8 --top-p 0.9\n\n\
        ## Sampling\n\n\
        | Mode | Temperature | Top P |\n|---|---|---|\n| thinking | 0.6 | 0.95 |\n| non-thinking | 0.7 | 0.8 |\n\n\
        | Parameter | Value |\n|---|---|\n| min_p | 0.05 |\n| repetition_penalty | 1.1 |\n";
    let all = parse_card(md);
    let find = |l: Label, t: f64| all.iter().find(|x| x.label == l && x.values.temperature == Some(t));
    assert!(find(Label::NonThinking, 0.7).is_some(), "{all:?}");
    assert_eq!(find(Label::Thinking, 0.6).unwrap().values.top_k, Some(40.0));
    assert_eq!(find(Label::General, 0.8).unwrap().values.top_p, Some(0.9));
    assert!(
        all.iter()
            .any(|x| x.label == Label::Thinking && x.values.top_p == Some(0.95))
    );
    assert!(
        all.iter()
            .any(|x| x.label == Label::NonThinking && x.values.top_p == Some(0.8))
    );
    let two_cols = all.iter().find(|x| x.values.min_p == Some(0.05)).unwrap();
    // Section « Sampling » sans mot de mode → général.
    assert_eq!(
        (two_cols.label, two_cols.values.repeat_penalty),
        (Label::General, Some(1.1))
    );
}

#[test]
fn bounds_and_irrelevant_sections() {
    let md = "## Usage\n\ntemperature=5 top_k=2.5 top_p=0.9\n\n## Citation\n\ntemperature=0.3\n";
    let all = parse_card(md);
    assert_eq!(all.len(), 1);
    assert_eq!(
        all[0].values,
        Sampling {
            top_p: Some(0.9),
            ..Default::default()
        }
    );
}

#[test]
fn merge_by_source_and_mode() {
    let inputs = SourceInputs {
        card_repo: Some(("https://hf/repo".into(), parse_card(&card("unsloth-qwen3-0.6b-gguf")))),
        card_base: None,
        generation_config: Some((
            "https://hf/gc".into(),
            from_generation_config(
                &serde_json::json!({"temperature": 0.6, "top_k": 20, "top_p": 0.95, "repetition_penalty": 1.05}),
            ),
        )),
        gguf: Sampling::default(),
        error: None,
    };
    let on = merge(&inputs, true);
    assert_eq!((on.values.temperature, on.values.top_p), (Some(0.6), Some(0.95)));
    assert_eq!(on.origins["temperature"].source, "card_repo");
    assert!(
        on.origins["temperature"]
            .excerpt
            .as_deref()
            .unwrap()
            .contains("Temperature=0.6")
    );
    assert_eq!(on.origins["repeat_penalty"].source, "generation_config");
    assert_eq!(on.values.repeat_penalty, Some(1.05));
    assert_eq!(on.origins["presence_penalty"].source, "defaut");
    let off = merge(&inputs, false);
    assert_eq!((off.values.temperature, off.values.top_p), (Some(0.7), Some(0.8)));
}

#[test]
fn contradiction_goes_to_choose_and_next_source() {
    let md = "## Recommended\n\n- Option A: temperature=0.2\n- Option B: temperature=0.9\n";
    let inputs = SourceInputs {
        card_repo: Some(("u".into(), parse_card(md))),
        card_base: Some(("b".into(), parse_card("## Usage\n\ntemperature=0.5\n"))),
        error: Some("réseau".into()),
        ..Default::default()
    };
    let m = merge(&inputs, true);
    assert_eq!(m.to_choose["temperature"].len(), 2);
    assert_eq!(m.values.temperature, Some(0.5));
    assert_eq!(m.origins["temperature"].source, "card_base");
    assert_eq!(m.error.as_deref(), Some("réseau"));
}
