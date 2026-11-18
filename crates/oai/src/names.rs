//! Noms d'outils à plat : `llama-server` ne connaît que des fonctions, Codex
//! regroupe les outils MCP en `namespace`. `<ns>__<nom>`, 64 caractères au
//! plus (55 premiers + `_` + 8 hexadécimaux du SHA-256 du nom complet),
//! suffixe `_2`, `_3` en cas de collision.

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

const MAX: usize = 64;

type Key = (Option<String>, String);

#[derive(Debug, Clone, Default, PartialEq)]
pub struct NameMap {
    by_flat: BTreeMap<String, Key>,
    by_key: BTreeMap<Key, String>,
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

fn base_name(ns: Option<&str>, name: &str) -> String {
    let full = match ns {
        Some(ns) => format!("{ns}__{name}"),
        None => name.to_string(),
    };
    if full.chars().count() <= MAX {
        return full;
    }
    let hash = Sha256::digest(full.as_bytes());
    let hex: String = hash.iter().take(4).map(|b| format!("{b:02x}")).collect();
    format!("{}_{hex}", truncate(&full, MAX - 9))
}

impl NameMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Nom à plat de `(ns, name)` ; le même à chaque appel.
    pub fn insert(&mut self, ns: Option<&str>, name: &str) -> String {
        let key = (ns.map(str::to_string), name.to_string());
        if let Some(flat) = self.by_key.get(&key) {
            return flat.clone();
        }
        let base = base_name(ns, name);
        let mut flat = base.clone();
        let mut k = 2;
        while self.by_flat.contains_key(&flat) {
            let suffix = format!("_{k}");
            flat = format!("{}{suffix}", truncate(&base, MAX - suffix.len()));
            k += 1;
        }
        self.by_flat.insert(flat.clone(), key.clone());
        self.by_key.insert(key, flat.clone());
        flat
    }

    /// `(namespace, nom)` d'origine ; un nom inconnu revient tel quel.
    pub fn split(&self, flat: &str) -> (Option<String>, String) {
        match self.by_flat.get(flat) {
            Some((ns, name)) => (ns.clone(), name.clone()),
            None => (None, flat.to_string()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.by_flat.is_empty()
    }
}
