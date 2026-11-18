//! Masquage des secrets : toute occurrence exacte d'une valeur du coffre (ou
//! d'une valeur injectée dans une sandbox vivante) de 6 caractères ou plus
//! devient `***`. Une valeur transformée (base64, coupée, casse changée) n'est
//! pas reconnue.
//!
//! Flux (deltas) : `Stream` retient la plus longue fin du texte en attente
//! qui est aussi le début d'une valeur, et ne coupe jamais au milieu d'une
//! valeur trouvée : une valeur coupée entre deux deltas ne sort pas en
//! morceaux.

use std::borrow::Cow;

use aho_corasick::{AhoCorasick, MatchKind};
use serde_json::Value;

/// Longueur minimale (en caractères) d'une valeur masquée.
pub const MIN_LEN: usize = 6;
pub const MASK: &str = "***";

#[derive(Default)]
pub struct Masker {
    ac: Option<AhoCorasick>,
    values: Vec<String>,
}

impl Masker {
    pub fn new(values: impl IntoIterator<Item = String>) -> Self {
        let mut values: Vec<String> = values.into_iter().filter(|v| v.chars().count() >= MIN_LEN).collect();
        values.sort();
        values.dedup();
        if values.is_empty() {
            return Self::default();
        }
        let ac = AhoCorasick::builder()
            .match_kind(MatchKind::LeftmostLongest)
            .build(&values)
            .expect("automate de masquage");
        Self { ac: Some(ac), values }
    }

    pub fn is_empty(&self) -> bool {
        self.ac.is_none()
    }

    pub fn mask<'a>(&self, s: &'a str) -> Cow<'a, str> {
        let Some(ac) = &self.ac else {
            return Cow::Borrowed(s);
        };
        if !ac.is_match(s) {
            return Cow::Borrowed(s);
        }
        let reps = vec![MASK; self.values.len()];
        Cow::Owned(ac.replace_all(s, &reps))
    }

    /// Masque chaque chaîne d'une valeur JSON (pas les clés).
    pub fn mask_json(&self, v: &mut Value) {
        if self.is_empty() {
            return;
        }
        match v {
            Value::String(s) => {
                if let Cow::Owned(m) = self.mask(s) {
                    *s = m;
                }
            }
            Value::Array(a) => a.iter_mut().for_each(|x| self.mask_json(x)),
            Value::Object(o) => o.values_mut().for_each(|x| self.mask_json(x)),
            _ => {}
        }
    }

    /// Point de coupe sûr de `s` (octets) : avant la plus longue fin qui est
    /// le début d'une valeur, et jamais au milieu d'une valeur trouvée.
    fn safe_cut(&self, s: &str) -> usize {
        let Some(ac) = &self.ac else {
            return s.len();
        };
        let mut cut = s.len();
        for v in &self.values {
            // Fins de `s` qui sont un début strict de `v`, de la plus longue à
            // la plus courte.
            let max = v.len().saturating_sub(1).min(s.len());
            for k in (1..=max).rev() {
                if !v.is_char_boundary(k) || !s.is_char_boundary(s.len() - k) {
                    continue;
                }
                if s.ends_with(&v[..k]) {
                    cut = cut.min(s.len() - k);
                    break;
                }
            }
        }
        for m in ac.find_iter(s) {
            if m.start() < cut && cut < m.end() {
                cut = m.start();
            }
        }
        cut
    }
}

/// État d'un flux de deltas (un par élément et par champ).
#[derive(Default, Debug)]
pub struct Stream {
    pending: String,
}

impl Stream {
    /// Ajoute `delta` ; rend le texte masqué à diffuser maintenant.
    pub fn push(&mut self, m: &Masker, delta: &str) -> String {
        self.pending.push_str(delta);
        let cut = m.safe_cut(&self.pending);
        let rest = self.pending.split_off(cut);
        let out = m.mask(&self.pending).into_owned();
        self.pending = rest;
        out
    }

    /// Fin du flux : tout ce qui reste, masqué.
    pub fn flush(&mut self, m: &Masker) -> String {
        let out = m.mask(&self.pending).into_owned();
        self.pending.clear();
        out
    }
}
