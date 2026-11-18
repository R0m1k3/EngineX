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

    /// Masque des octets (sortie d'un terminal, peut ne pas être de l'UTF-8).
    pub fn mask_bytes<'a>(&self, b: &'a [u8]) -> Cow<'a, [u8]> {
        let Some(ac) = &self.ac else {
            return Cow::Borrowed(b);
        };
        if !ac.is_match(b) {
            return Cow::Borrowed(b);
        }
        let reps = vec![MASK.as_bytes(); self.values.len()];
        Cow::Owned(ac.replace_all_bytes(b, &reps))
    }

    /// Point de coupe sûr de `s` : avant la plus longue fin qui est le début
    /// d'une valeur, et jamais au milieu d'une valeur trouvée. Sur un texte
    /// UTF-8 valide, la coupe tombe toujours entre deux caractères (les
    /// valeurs sont de l'UTF-8 valide).
    fn safe_cut(&self, s: &[u8]) -> usize {
        let Some(ac) = &self.ac else {
            return s.len();
        };
        let mut cut = s.len();
        for v in &self.values {
            let v = v.as_bytes();
            // Fins de `s` qui sont un début strict de `v`, de la plus longue à
            // la plus courte.
            let max = v.len().saturating_sub(1).min(s.len());
            for k in (1..=max).rev() {
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

/// État d'un flux d'octets (terminal) : cœur du masquage en flux.
#[derive(Default, Debug)]
pub struct ByteStream {
    pending: Vec<u8>,
}

impl ByteStream {
    /// Ajoute `delta` ; rend les octets masqués à diffuser maintenant.
    pub fn push(&mut self, m: &Masker, delta: &[u8]) -> Vec<u8> {
        self.pending.extend_from_slice(delta);
        let cut = m.safe_cut(&self.pending);
        let rest = self.pending.split_off(cut);
        let out = m.mask_bytes(&self.pending).into_owned();
        self.pending = rest;
        out
    }

    /// Fin du flux : tout ce qui reste, masqué.
    pub fn flush(&mut self, m: &Masker) -> Vec<u8> {
        let out = m.mask_bytes(&self.pending).into_owned();
        self.pending.clear();
        out
    }
}

/// État d'un flux de deltas texte (un par élément et par champ), au-dessus
/// de `ByteStream`.
#[derive(Default, Debug)]
pub struct Stream {
    inner: ByteStream,
}

impl Stream {
    /// Ajoute `delta` ; rend le texte masqué à diffuser maintenant.
    pub fn push(&mut self, m: &Masker, delta: &str) -> String {
        String::from_utf8_lossy(&self.inner.push(m, delta.as_bytes())).into_owned()
    }

    /// Fin du flux : tout ce qui reste, masqué.
    pub fn flush(&mut self, m: &Masker) -> String {
        String::from_utf8_lossy(&self.inner.flush(m)).into_owned()
    }
}
