//! Réglages d'échantillonnage conseillés pour un modèle : lus dans sa model
//! card (README du dépôt, puis du modèle de base), dans `generation_config.json`
//! et dans les clés `general.sampling.*` du GGUF, puis fusionnés paramètre par
//! paramètre dans cet ordre. Le parseur de card est déterministe : quand la
//! card se contredit, la valeur est laissée « à choisir ».

use std::{collections::BTreeMap, sync::LazyLock};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::gguf::GgufMeta;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Sampling {
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub top_k: Option<f64>,
    pub min_p: Option<f64>,
    pub presence_penalty: Option<f64>,
    pub repeat_penalty: Option<f64>,
}

/// Noms des paramètres, dans l'ordre d'affichage.
pub const PARAMS: [&str; 6] = [
    "temperature",
    "top_p",
    "top_k",
    "min_p",
    "presence_penalty",
    "repeat_penalty",
];

impl Sampling {
    pub fn get(&self, p: &str) -> Option<f64> {
        match p {
            "temperature" => self.temperature,
            "top_p" => self.top_p,
            "top_k" => self.top_k,
            "min_p" => self.min_p,
            "presence_penalty" => self.presence_penalty,
            "repeat_penalty" => self.repeat_penalty,
            _ => None,
        }
    }
    fn slot(&mut self, p: &str) -> Option<&mut Option<f64>> {
        Some(match p {
            "temperature" => &mut self.temperature,
            "top_p" => &mut self.top_p,
            "top_k" => &mut self.top_k,
            "min_p" => &mut self.min_p,
            "presence_penalty" => &mut self.presence_penalty,
            "repeat_penalty" => &mut self.repeat_penalty,
            _ => return None,
        })
    }
    /// Pose `v` si elle est dans les bornes du paramètre (sinon ignorée).
    pub fn set(&mut self, p: &str, v: f64) {
        if valid(p, v)
            && let Some(slot) = self.slot(p)
        {
            *slot = Some(v);
        }
    }
    pub fn is_empty(&self) -> bool {
        PARAMS.iter().all(|p| self.get(p).is_none())
    }
}

fn valid(p: &str, v: f64) -> bool {
    match p {
        "temperature" => (0.0..=2.0).contains(&v),
        "top_p" | "min_p" => (0.0..=1.0).contains(&v),
        "top_k" => (0.0..=200.0).contains(&v) && v.fract() == 0.0,
        "presence_penalty" | "repeat_penalty" => (0.0..=2.0).contains(&v),
        _ => false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Label {
    Thinking,
    NonThinking,
    General,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CandidateSet {
    pub label: Label,
    pub values: Sampling,
    pub excerpt: String,
}

/// Mots qui rendent une section ou un paragraphe pertinent.
const KEYWORDS: [&str; 8] = [
    "sampling",
    "best practice",
    "recommended",
    "usage",
    "parameters",
    "inference",
    "settings",
    "generation",
];
const NON_THINKING: [&str; 8] = [
    "non-thinking",
    "non thinking",
    "no-think",
    "no think",
    "/no_think",
    "without thinking",
    "thinking off",
    "instruct mode",
];
const THINKING: [&str; 3] = ["thinking", "reasoning", "think"];

/// Nom de paramètre écrit dans une card → nom canonique.
fn canonical(name: &str) -> Option<&'static str> {
    let n: String = name
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    Some(match n.as_str() {
        "temperature" | "temp" => "temperature",
        "topp" => "top_p",
        "topk" => "top_k",
        "minp" => "min_p",
        "presencepenalty" => "presence_penalty",
        "repetitionpenalty" | "repeatpenalty" => "repeat_penalty",
        _ => return None,
    })
}

const NAMES: &str = r"temperature|temp|top[_\- ]?p|top[_\- ]?k|min[_\- ]?p|presence[_\- ]penalty|repetition[_\- ]penalty|repeat[_\- ]penalty";

/// `nom=valeur`, `nom: valeur`, `"nom": valeur`.
static ASSIGN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r#"(?i)(?:^|[^a-z0-9_\-])["'`]?({NAMES})["'`]?\s*[=:]\s*["'`]?(-?[0-9]+(?:\.[0-9]+)?)"#
    ))
    .expect("motif")
});
/// Arguments de llama.cpp : `--temp 0.6`, `--top-p 0.95`...
static FLAG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)--(temp|top-p|top-k|min-p|presence-penalty|repeat-penalty)[ =]+(-?[0-9]+(?:\.[0-9]+)?)")
        .expect("motif")
});

fn values_in(text: &str) -> Sampling {
    let mut s = Sampling::default();
    for re in [&*ASSIGN, &*FLAG] {
        for c in re.captures_iter(text) {
            if let (Some(p), Ok(v)) = (canonical(&c[1]), c[2].parse::<f64>()) {
                s.set(p, v);
            }
        }
    }
    s
}

/// Étiquette d'un texte, `None` s'il ne parle d'aucun mode.
fn label_of(text: &str) -> Option<Label> {
    let t = text.to_ascii_lowercase();
    if NON_THINKING.iter().any(|k| t.contains(k)) {
        Some(Label::NonThinking)
    } else if THINKING.iter().any(|k| t.contains(k)) {
        Some(Label::Thinking)
    } else {
        None
    }
}

fn relevant(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    KEYWORDS.iter().any(|k| t.contains(k))
}

fn excerpt(text: &str) -> String {
    let t: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match t.char_indices().nth(300) {
        Some((i, _)) => format!("{}…", &t[..i]),
        None => t,
    }
}

fn cells(row: &str) -> Vec<String> {
    row.trim()
        .trim_matches('|')
        .split('|')
        .map(|c| c.trim().to_string())
        .collect()
}

fn is_separator(row: &str) -> bool {
    let r = row.trim();
    r.starts_with('|') && r.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

struct Ctx<'a> {
    headings: &'a [String],
}

impl Ctx<'_> {
    fn path(&self) -> String {
        self.headings.join(" / ")
    }
    fn set(&self, text: &str, header: Option<&str>, values: Sampling) -> Option<CandidateSet> {
        if values.is_empty() {
            return None;
        }
        let label = label_of(text)
            .or_else(|| header.and_then(label_of))
            .or_else(|| self.headings.iter().rev().find_map(|h| label_of(h)))
            .unwrap_or(Label::General);
        Some(CandidateSet {
            label,
            values,
            excerpt: excerpt(text),
        })
    }
}

/// Tableau Markdown : en-tête nommant des paramètres (une ligne = un jeu), ou
/// deux colonnes « paramètre | valeur » (tout le tableau = un jeu).
fn table_sets(rows: &[&str], ctx: &Ctx, out: &mut Vec<CandidateSet>) {
    let header = cells(rows[0]);
    let data: Vec<&&str> = rows.iter().skip(1).filter(|r| !is_separator(r)).collect();
    let cols: Vec<(usize, &'static str)> = header
        .iter()
        .enumerate()
        .filter_map(|(i, h)| canonical(h).map(|p| (i, p)))
        .collect();
    if !cols.is_empty() {
        for row in data {
            let c = cells(row);
            let mut s = Sampling::default();
            for &(i, p) in &cols {
                if let Some(v) = c.get(i).and_then(|x| x.trim_matches('`').parse::<f64>().ok()) {
                    s.set(p, v);
                }
            }
            out.extend(ctx.set(row, Some(rows[0]), s));
        }
        return;
    }
    let mut s = Sampling::default();
    for row in &data {
        let c = cells(row);
        if let (Some(p), Some(v)) = (
            c.first().and_then(|n| canonical(n.trim_matches('`'))),
            c.get(1).and_then(|x| x.trim_matches('`').parse::<f64>().ok()),
        ) {
            s.set(p, v);
        }
    }
    let text = rows.join("\n");
    out.extend(ctx.set(&text, Some(rows[0]), s));
}

/// Jeux de réglages trouvés dans une card Markdown.
pub fn parse_card(markdown: &str) -> Vec<CandidateSet> {
    let mut out = Vec::new();
    let mut headings: Vec<(usize, String)> = Vec::new();
    let mut block: Vec<&str> = Vec::new();
    let mut in_code = false;

    let flush = |block: &mut Vec<&str>, headings: &[(usize, String)], out: &mut Vec<CandidateSet>| {
        if block.is_empty() {
            return;
        }
        let hs: Vec<String> = headings.iter().map(|(_, h)| h.clone()).collect();
        let ctx = Ctx { headings: &hs };
        let text = block.join("\n");
        let section_ok = relevant(&ctx.path());
        if block[0].trim_start().starts_with('|') && block.len() >= 2 {
            if section_ok || relevant(&text) {
                table_sets(block, &ctx, out);
            }
        } else if section_ok || relevant(&text) {
            // Une ligne de liste = un jeu ; sinon le paragraphe entier.
            let items: Vec<&&str> = block
                .iter()
                .filter(|l| {
                    let t = l.trim_start();
                    t.starts_with("- ")
                        || t.starts_with("* ")
                        || t.chars().next().is_some_and(|c| c.is_ascii_digit()) && t.contains(". ")
                })
                .collect();
            if items.len() > 1 {
                for l in items {
                    out.extend(ctx.set(l, None, values_in(l)));
                }
            } else {
                out.extend(ctx.set(&text, None, values_in(&text)));
            }
        }
        block.clear();
    };

    for line in markdown.lines() {
        let t = line.trim_start();
        if t.starts_with("```") {
            in_code = !in_code;
            // Un bloc de code est un paragraphe à lui seul.
            flush(&mut block, &headings, &mut out);
            continue;
        }
        if !in_code && t.starts_with('#') {
            flush(&mut block, &headings, &mut out);
            let level = t.chars().take_while(|c| *c == '#').count();
            headings.retain(|(l, _)| *l < level);
            headings.push((level, t.trim_start_matches('#').trim().to_string()));
            continue;
        }
        if !in_code && t.trim_start_matches('>').trim().is_empty() {
            flush(&mut block, &headings, &mut out);
            continue;
        }
        let is_table = t.starts_with('|');
        if !block.is_empty() && block[0].trim_start().starts_with('|') != is_table {
            flush(&mut block, &headings, &mut out);
        }
        block.push(line);
    }
    flush(&mut block, &headings, &mut out);
    out
}

pub fn from_generation_config(v: &serde_json::Value) -> Sampling {
    let mut s = Sampling::default();
    for (k, p) in [
        ("temperature", "temperature"),
        ("top_p", "top_p"),
        ("top_k", "top_k"),
        ("min_p", "min_p"),
        ("presence_penalty", "presence_penalty"),
        ("repetition_penalty", "repeat_penalty"),
    ] {
        if let Some(x) = v.get(k).and_then(serde_json::Value::as_f64) {
            s.set(p, x);
        }
    }
    s
}

pub fn from_gguf(meta: &GgufMeta) -> Sampling {
    let mut s = Sampling::default();
    for (k, p) in [
        ("temp", "temperature"),
        ("top_p", "top_p"),
        ("top_k", "top_k"),
        ("min_p", "min_p"),
        ("penalty_present", "presence_penalty"),
        ("penalty_repeat", "repeat_penalty"),
    ] {
        if let Some(x) = meta.sampling.get(k) {
            s.set(p, *x);
        }
    }
    s
}

#[derive(Clone, Debug, Default)]
pub struct SourceInputs {
    /// (url, jeux) de la card du dépôt téléchargé.
    pub card_repo: Option<(String, Vec<CandidateSet>)>,
    /// (url, jeux) de la card du modèle de base.
    pub card_base: Option<(String, Vec<CandidateSet>)>,
    pub generation_config: Option<(String, Sampling)>,
    pub gguf: Sampling,
    /// Erreur rencontrée en lisant les sources (réseau...), montrée à l'UI.
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Origin {
    /// `card_repo`, `card_base`, `generation_config`, `gguf` ou `defaut`.
    pub source: String,
    pub excerpt: Option<String>,
    pub url: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Merged {
    pub values: Sampling,
    pub origins: BTreeMap<String, Origin>,
    /// Paramètres que la card donne de plusieurs façons : jeux à départager.
    pub to_choose: BTreeMap<String, Vec<CandidateSet>>,
    pub error: Option<String>,
}

/// Jeux retenus d'une card pour le mode voulu : étiquette du mode, sinon
/// « général ».
fn pick(sets: &[CandidateSet], reasoning_on: bool) -> Vec<&CandidateSet> {
    let want = if reasoning_on {
        Label::Thinking
    } else {
        Label::NonThinking
    };
    let of = |l: Label| sets.iter().filter(|s| s.label == l).collect::<Vec<_>>();
    let w = of(want);
    if w.is_empty() { of(Label::General) } else { w }
}

pub fn merge(inputs: &SourceInputs, reasoning_on: bool) -> Merged {
    let mut m = Merged {
        error: inputs.error.clone(),
        ..Default::default()
    };
    let cards = [("card_repo", &inputs.card_repo), ("card_base", &inputs.card_base)];
    for p in PARAMS {
        let mut found = false;
        for (name, card) in cards {
            let Some((url, sets)) = card else { continue };
            let chosen: Vec<&CandidateSet> = pick(sets, reasoning_on)
                .into_iter()
                .filter(|s| s.values.get(p).is_some())
                .collect();
            let Some(first) = chosen.first() else { continue };
            let v = first.values.get(p);
            if chosen.iter().any(|s| s.values.get(p) != v) {
                // Contradiction : à choisir ; la valeur vient d'une source suivante.
                m.to_choose
                    .entry(p.to_string())
                    .or_insert_with(|| chosen.iter().map(|s| (*s).clone()).collect());
                continue;
            }
            m.values.set(p, v.unwrap_or_default());
            m.origins.insert(
                p.to_string(),
                Origin {
                    source: name.into(),
                    excerpt: Some(first.excerpt.clone()),
                    url: Some(url.clone()),
                },
            );
            found = true;
            break;
        }
        if found {
            continue;
        }
        let rest = [
            (
                "generation_config",
                inputs.generation_config.as_ref().map(|(u, s)| (Some(u.clone()), s)),
            ),
            ("gguf", Some((None, &inputs.gguf))),
        ];
        for (name, src) in rest {
            if let Some((url, s)) = src
                && let Some(v) = s.get(p)
            {
                m.values.set(p, v);
                m.origins.insert(
                    p.to_string(),
                    Origin {
                        source: name.into(),
                        excerpt: None,
                        url,
                    },
                );
                found = true;
                break;
            }
        }
        if !found {
            m.origins.insert(
                p.to_string(),
                Origin {
                    source: "defaut".into(),
                    excerpt: None,
                    url: None,
                },
            );
        }
    }
    m
}
