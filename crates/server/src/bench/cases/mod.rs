//! Les 15 tests de la page Tester.
//!
//! Trois familles :
//! - **visuels** : une page web ouverte dans le Chromium d'EngineX (bureau et
//!   téléphone), contrôlée par des scénarios réels ; Michael note le visuel ;
//! - **code** : contrôlés à 100 % par la machine, avec des tests cachés copiés
//!   seulement à l'audit ;
//! - **outils** : le résultat et la façon de travailler (journal de la session).
//!
//! Chaque contrôle vaut des points ; un contrôle *essentiel* raté fait échouer
//! le test. Les solutions de référence (`refs/`) passent tous les contrôles
//! (`scripts/bench-refs.py`).

mod code;
mod tools;
mod visual;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Family {
    Visual,
    Code,
    Tools,
}

impl Family {
    pub fn key(self) -> &'static str {
        match self {
            Family::Visual => "visual",
            Family::Code => "code",
            Family::Tools => "tools",
        }
    }
}

/// Lecture du journal de la session : `Ok(preuve)` ou `Err(raison)`.
/// Fichiers de départ produits par une fonction : (chemin, contenu).
pub type Generator = fn() -> Vec<(String, String)>;

pub type LogRule = fn(&crate::bench::journal::Journal) -> Result<String, String>;

pub enum Kind {
    /// `bash -lc` dans la sandbox, dans le dossier de la session (code 0 = OK).
    /// `$BENCH_DIR` = ce dossier ; `$BENCH_HIDDEN` = dossier des tests cachés.
    Shell(&'static str),
    /// Page ouverte dans Chromium (`path` peut porter une requête). Chaque étape
    /// est le corps d'une fonction JS asynchrone qui lève une erreur si KO ;
    /// l'étape `@reload` recharge la page (localStorage gardé).
    Page {
        path: &'static str,
        phone: bool,
        steps: &'static [&'static str],
    },
    /// Aucune erreur de console pendant les contrôles `Page` du test.
    ConsoleClean,
    /// Règle sur le journal de la session (commandes, outils appelés).
    Log(LogRule),
}

pub struct Check {
    pub name: &'static str,
    pub points: u32,
    pub essential: bool,
    pub kind: Kind,
}

pub struct Case {
    pub key: &'static str,
    pub title: &'static str,
    pub family: Family,
    /// Ce que la machine contrôle, en une phrase (liste des tests).
    pub summary: &'static str,
    pub prompt: &'static str,
    /// Fichiers donnés au modèle.
    pub files: &'static [(&'static str, &'static str)],
    /// Fichiers donnés au modèle, produits par une fonction (gros fichiers).
    pub generated: Option<Generator>,
    /// Fichiers fournis remis tels quels avant les contrôles.
    pub protected: &'static [&'static str],
    /// Tests cachés : écrits dans `$BENCH_HIDDEN` au moment de l'audit seulement.
    pub hidden: &'static [(&'static str, &'static str)],
    /// Commande lancée dans la sandbox avant le tour (dépôt git de départ…).
    pub setup: Option<&'static str>,
    /// Page capturée au bureau et au téléphone (tests avec rendu).
    pub page: Option<&'static str>,
    pub checks: &'static [Check],
    /// Plafond de temps du tour (secondes, ≤ 1200).
    pub timeout_s: u64,
}

impl Case {
    pub fn points_max(&self) -> u32 {
        self.checks.iter().map(|c| c.points).sum()
    }

    /// Tous les fichiers de départ (fixes et générés).
    pub fn start_files(&self) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = self.files.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        if let Some(g) = self.generated {
            v.extend(g());
        }
        v
    }

    pub fn needs_browser(&self) -> bool {
        self.checks.iter().any(|c| matches!(c.kind, Kind::Page { .. })) || self.page.is_some()
    }
}

pub fn all() -> &'static [&'static Case] {
    &ALL
}

pub fn get(key: &str) -> Option<&'static Case> {
    ALL.iter().copied().find(|c| c.key == key)
}

static ALL: [&Case; 15] = [
    &visual::LANDING,
    &visual::CALCULATOR,
    &visual::TODO,
    &visual::DASHBOARD,
    &visual::SNAKE,
    &visual::SIGNUP,
    &visual::CALENDAR,
    &code::BUGFIX,
    &code::DATES,
    &code::REST_API,
    &code::REFACTOR,
    &code::MULTI_FILE,
    &tools::BIG_LOG,
    &tools::BROWSER_BUG,
    &tools::REPO_HUNT,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cases_are_well_formed() {
        let mut keys = std::collections::HashSet::new();
        for c in all() {
            assert!(keys.insert(c.key), "clé en double : {}", c.key);
            assert!(!c.checks.is_empty(), "{} : aucun contrôle", c.key);
            assert!(
                c.checks.iter().any(|k| k.essential),
                "{} : aucun contrôle essentiel",
                c.key
            );
            assert!(c.timeout_s <= 1200 && c.timeout_s >= 300, "{} : plafond", c.key);
            let files = c.start_files();
            for p in c.protected {
                assert!(files.iter().any(|(r, _)| r == p), "{} : {p} protégé mais absent", c.key);
            }
            if c.family == Family::Visual {
                assert!(c.page.is_some(), "{} : test visuel sans page", c.key);
            }
            assert!(c.points_max() >= 10, "{} : trop peu de points", c.key);
        }
        assert_eq!(all().len(), 15);
    }

    /// `BENCH_DUMP=/chemin.json cargo test -p enginex-server --lib dump_for_refs -- --ignored` :
    /// les tests au format lu par `scripts/bench-refs.py`.
    #[test]
    #[ignore]
    fn dump_for_refs() {
        use serde_json::json;
        let out: Vec<_> = all()
            .iter()
            .map(|c| {
                let checks: Vec<_> = c
                    .checks
                    .iter()
                    .map(|k| match &k.kind {
                        Kind::Shell(s) => json!({"name": k.name, "type": "shell", "script": s}),
                        Kind::Page { path, phone, steps } => json!({"name": k.name, "type": "page", "path": path, "phone": phone,
                            "steps": steps.iter().map(|s| if s.starts_with('@') { s.to_string() } else { crate::bench::audit::wrap_step(s) }).collect::<Vec<_>>()}),
                        Kind::ConsoleClean => json!({"name": k.name, "type": "console"}),
                        Kind::Log(_) => json!({"name": k.name, "type": "log"}),
                    })
                    .zip(c.checks.iter())
                    .map(|(mut v, k)| {
                        v["points"] = json!(k.points);
                        v["essential"] = json!(k.essential);
                        v
                    })
                    .collect();
                json!({"key": c.key, "family": c.family.key(), "files": c.start_files(), "hidden": c.hidden, "setup": c.setup,
                    "page": c.page, "checks": checks, "benchh": include_str!("../data/hidden/benchh.py")})
            })
            .collect();
        std::fs::write(
            std::env::var("BENCH_DUMP").expect("BENCH_DUMP"),
            serde_json::to_string(&out).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn generated_files_are_stable() {
        for c in all() {
            if let Some(g) = c.generated {
                assert_eq!(g(), g(), "{} : génération non déterministe", c.key);
            }
        }
    }
}
