//! Le journal d'un test : ce que le modèle a lancé (commandes, outils), lu dans
//! `chat_items`, et les règles qui le contrôlent (tests d'outils, méthode).

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

#[derive(Debug, Clone)]
pub enum Item {
    Command {
        command: String,
        exit: Option<i64>,
        /// `commandActions` de Codex : (type, chemin ou requête).
        actions: Vec<(String, String)>,
    },
    Tool {
        name: String,
    },
    FileChange,
}

#[derive(Default, Debug)]
pub struct Journal {
    pub items: Vec<Item>,
    /// Fichiers de test fournis que le modèle a modifiés (vu avant leur remise).
    pub protected_changed: Vec<String>,
}

impl Journal {
    /// Depuis les lignes `(kind, data)` de `chat_items`, dans l'ordre.
    pub fn from_rows(rows: &[(String, String)]) -> Self {
        let mut items = Vec::new();
        for (kind, data) in rows {
            let Ok(v) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            match kind.as_str() {
                "commandExecution" => items.push(Item::Command {
                    command: v["command"].as_str().unwrap_or("").to_string(),
                    exit: v["exitCode"].as_i64(),
                    actions: v["commandActions"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|a| {
                            let t = a["type"].as_str().unwrap_or("").to_string();
                            let what = a["path"].as_str().or_else(|| a["query"].as_str()).unwrap_or("");
                            (t, what.to_string())
                        })
                        .collect(),
                }),
                "mcpToolCall" => items.push(Item::Tool {
                    name: v["tool"].as_str().unwrap_or("").to_string(),
                }),
                "fileChange" => items.push(Item::FileChange),
                _ => {}
            }
        }
        Self {
            items,
            protected_changed: Vec::new(),
        }
    }

    fn commands(&self) -> impl Iterator<Item = (usize, &str, Option<i64>, &[(String, String)])> {
        self.items.iter().enumerate().filter_map(|(i, it)| match it {
            Item::Command { command, exit, actions } => Some((i, command.as_str(), *exit, actions.as_slice())),
            _ => None,
        })
    }

    /// Rang de la dernière écriture de fichier du projet (`None` : aucune).
    pub fn last_write(&self) -> Option<usize> {
        self.items.iter().enumerate().rev().find_map(|(i, it)| match it {
            Item::FileChange => Some(i),
            Item::Command { command, .. } if writes_files(command) => Some(i),
            _ => None,
        })
    }

    fn tools(&self) -> impl Iterator<Item = (usize, &str)> {
        self.items.iter().enumerate().filter_map(|(i, it)| match it {
            Item::Tool { name } => Some((i, name.as_str())),
            _ => None,
        })
    }
}

static REDIRECT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|[^0-9&>])>{1,2}\s*([^\s;&|)]+)").unwrap());
static WRITE_CMD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\btee\b|\bsed\s+(-[a-zA-Z]*\s+)*-i|\bperl\s+-[a-z]*i|apply_patch|\bopen\([^)]*['"][wa]['"]|write_text\(|\b(mv|cp|rm|touch)\s|\bgit\s+(checkout|restore|apply|reset)\b"#).unwrap()
});
static TEST_RUN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"unittest|pytest|\bnpm\s+(run\s+)?test\b|node\s+--test|python3?\s+\S*test_\w+\.py").unwrap()
});
static SEARCH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(rg|grep|egrep|ag|ack|find|git\s+grep|fd)\b").unwrap());
static READ_CMD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(cat|head|tail|less|more|nl|sed\s+-n)\s+[^|;&]*?([\w./-]+\.py)\b").unwrap());
static LOG_DUMP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\b(cat|nl|less|more|tac)\s+(\S*/)?access\.log\b\s*['"]?\s*($|;|&&|\|\|)"#).unwrap());

/// La commande écrit-elle un fichier du projet ? (redirection hors /dev et
/// /tmp, `tee`, `sed -i`, écriture Python, `mv`/`cp`/`rm`…)
pub fn writes_files(cmd: &str) -> bool {
    // Le commit et la lecture de l'état git ne sont pas des modifications.
    let cmd_wo_git = cmd.replace("git commit", "").replace("git add", "");
    if WRITE_CMD.is_match(&cmd_wo_git) {
        return true;
    }
    REDIRECT.captures_iter(&cmd_wo_git).any(|c| {
        let t = c[1].trim_matches(|ch| ch == '"' || ch == '\'');
        !(t.starts_with("/dev/") || t.starts_with("/tmp/") || t.starts_with('&') || t.is_empty())
    })
}

pub fn is_test_run(cmd: &str) -> bool {
    TEST_RUN.is_match(cmd)
}

fn short(cmd: &str) -> String {
    let c: String = cmd.chars().take(140).collect();
    if cmd.chars().count() > 140 {
        format!("{c}…")
    } else {
        c
    }
}

// ---- Règles ----

pub fn protected_untouched(j: &Journal) -> Result<String, String> {
    if j.protected_changed.is_empty() {
        Ok("fichiers de test intacts".into())
    } else {
        Err(format!(
            "modifiés : {} (remis d'origine avant les contrôles)",
            j.protected_changed.join(", ")
        ))
    }
}

/// Dernière exécution des tests au vert, après la dernière modification.
pub fn ran_tests_green(j: &Journal) -> Result<String, String> {
    let runs: Vec<(usize, &str, Option<i64>)> = j
        .commands()
        .filter(|(_, c, _, _)| is_test_run(c))
        .map(|(i, c, e, _)| (i, c, e))
        .collect();
    let Some(&(i, cmd, exit)) = runs.last() else {
        return Err("n'a jamais lancé les tests".into());
    };
    if exit != Some(0) {
        return Err(format!(
            "dernier passage des tests en échec (code {exit:?}) : {}",
            short(cmd)
        ));
    }
    match j.last_write() {
        Some(w) if w > i => Err(format!(
            "fichiers modifiés après le dernier passage des tests ({} lancements en tout)",
            runs.len()
        )),
        _ => Ok(format!(
            "{} lancement(s) des tests, le dernier au vert : {}",
            runs.len(),
            short(cmd)
        )),
    }
}

pub fn used_curl(j: &Journal) -> Result<String, String> {
    let n = j.commands().filter(|(_, c, _, _)| c.contains("curl ")).count();
    if n > 0 {
        Ok(format!("{n} commande(s) curl"))
    } else {
        Err("aucun curl lancé".into())
    }
}

pub fn opened_browser(j: &Journal) -> Result<String, String> {
    let n = j.tools().filter(|(_, t)| *t == "browser_open").count();
    if n > 0 {
        Ok(format!("browser_open × {n}"))
    } else {
        Err("browser_open jamais appelé".into())
    }
}

pub fn interacted_in_browser(j: &Journal) -> Result<String, String> {
    let used: Vec<&str> = j
        .tools()
        .map(|(_, t)| t)
        .filter(|t| {
            matches!(
                *t,
                "browser_click" | "browser_type" | "browser_click_xy" | "browser_key"
            )
        })
        .collect();
    if used.is_empty() {
        Err("aucun clic ni saisie dans la page".into())
    } else {
        Ok(format!("{} action(s) : {}", used.len(), dedup(&used).join(", ")))
    }
}

pub fn browser_after_last_write(j: &Journal) -> Result<String, String> {
    let Some(w) = j.last_write() else {
        return Err("aucune modification de fichier".into());
    };
    let after: Vec<&str> = j
        .tools()
        .filter(|(i, t)| *i > w && t.starts_with("browser_"))
        .map(|(_, t)| t)
        .collect();
    if after.iter().any(|t| *t != "browser_screenshot") {
        Ok(format!("après la dernière modification : {}", dedup(&after).join(", ")))
    } else {
        Err("pas de navigateur après la dernière modification".into())
    }
}

pub fn searched(j: &Journal) -> Result<String, String> {
    let n = j
        .commands()
        .filter(|(_, c, _, a)| a.iter().any(|(t, _)| t == "search") || SEARCH.is_match(c))
        .count();
    if n > 0 {
        Ok(format!("{n} recherche(s)"))
    } else {
        Err("aucune recherche (rg, grep, find…)".into())
    }
}

pub fn few_files_read(j: &Journal) -> Result<String, String> {
    let mut files: Vec<String> = Vec::new();
    for (_, c, _, a) in j.commands() {
        for (t, p) in a {
            if t == "read" && !p.is_empty() {
                files.push(p.rsplit('/').next().unwrap_or(p).to_string());
            }
        }
        for cap in READ_CMD.captures_iter(c) {
            files.push(cap[2].rsplit('/').next().unwrap_or(&cap[2]).to_string());
        }
    }
    files.sort();
    files.dedup();
    if files.len() <= 12 {
        Ok(format!("{} fichier(s) lu(s)", files.len()))
    } else {
        Err(format!("{} fichiers lus un par un", files.len()))
    }
}

pub fn no_full_dump_of_log(j: &Journal) -> Result<String, String> {
    match j.commands().find(|(_, c, _, _)| LOG_DUMP.is_match(c)) {
        Some((_, c, _, _)) => Err(format!("journal affiché en entier : {}", short(c))),
        None => Ok("lu par morceaux ou par script".into()),
    }
}

fn dedup<'a>(v: &[&'a str]) -> Vec<&'a str> {
    let mut out: Vec<&str> = Vec::new();
    for x in v {
        if !out.contains(x) {
            out.push(x);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(c: &str, exit: i64) -> Item {
        Item::Command {
            command: c.into(),
            exit: Some(exit),
            actions: vec![],
        }
    }

    #[test]
    fn writes_are_seen() {
        for c in [
            "/bin/bash -lc 'cat > cart.py <<EOF\nx\nEOF'",
            "sed -i 's/a/b/' x.py",
            "echo hi >> notes.txt",
            "python3 - <<'PY'\nopen('a.py','w').write('x')\nPY",
            "tee out.py",
        ] {
            assert!(writes_files(c), "{c}");
        }
        for c in [
            "python3 -m unittest -v 2>&1",
            "ls > /dev/null",
            "curl -s localhost:8000 > /tmp/o.json",
            "git commit -qm 'fix: x'",
            "grep -rn TVA shop",
            "cat x.py",
            "python3 -c 'print(1>0)'",
        ] {
            assert!(!writes_files(c), "{c}");
        }
    }

    #[test]
    fn tests_after_last_write() {
        let j = Journal {
            items: vec![
                cmd("python3 -m unittest", 1),
                cmd("sed -i s/a/b/ cart.py", 0),
                cmd("python3 -m unittest -v", 0),
            ],
            ..Default::default()
        };
        assert!(ran_tests_green(&j).is_ok());
        let j = Journal {
            items: vec![cmd("python3 -m unittest", 0), cmd("sed -i s/a/b/ cart.py", 0)],
            ..Default::default()
        };
        assert!(ran_tests_green(&j).is_err());
        assert!(ran_tests_green(&Journal::default()).is_err());
    }

    #[test]
    fn browser_rules() {
        let j = Journal {
            items: vec![
                Item::Tool {
                    name: "browser_open".into(),
                },
                cmd("sed -i s/add-cart/add-to-cart/ site/app.js", 0),
                Item::Tool {
                    name: "browser_click".into(),
                },
            ],
            ..Default::default()
        };
        assert!(opened_browser(&j).is_ok());
        assert!(interacted_in_browser(&j).is_ok());
        assert!(browser_after_last_write(&j).is_ok());
    }

    #[test]
    fn log_dump() {
        let j = |c: &str| Journal {
            items: vec![cmd(c, 0)],
            ..Default::default()
        };
        assert!(no_full_dump_of_log(&j("/bin/bash -lc 'cat access.log'")).is_err());
        assert!(no_full_dump_of_log(&j("cat access.log | wc -l")).is_ok());
        assert!(no_full_dump_of_log(&j("head -5 access.log")).is_ok());
    }
}
