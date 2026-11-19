//! Allègement de l'historique envoyé à `llama-server` (gros fichiers).
//!
//! Codex garde chaque appel d'outil en entier jusqu'au compactage, qui efface
//! tout le travail du tour. Un fichier écrit par un heredoc (`cat > f <<'EOF'`)
//! occupe donc le contexte deux fois plus longtemps qu'utile et fait compacter
//! au milieu de l'écriture : le modèle perd ce qu'il a écrit et recommence.
//!
//! Ici, sur les appels déjà passés (tout sauf la dernière étape) :
//! - un appel qui écrit un gros heredoc est remplacé par un résumé
//!   (`enginex_resume` : la commande sans le contenu, et le nombre de lignes) ;
//! - dans le tour en cours, une note s'ajoute au résultat de la 3e écriture
//!   complète d'un même fichier, et aux paliers de 72, 144, 216… appels
//!   (repris de `budget_nudge` de Loki, mode Code).
//!
//! Tout est déterministe et ne dépend que de l'historique : une même étape
//! donne toujours les mêmes octets, le cache de prompt sert d'une requête à
//! l'autre (seule l'étape qui cesse d'être la dernière est relue).

use std::collections::HashMap;

use serde_json::{Value, json};

/// Un heredoc est allégé à partir de ce nombre de lignes…
pub const SHRINK_MIN_LINES: usize = 30;
/// … ou de cette taille (caractères).
pub const SHRINK_MIN_CHARS: usize = 2000;
/// Écritures complètes d'un même fichier dans un tour avant la note.
pub const REWRITE_NOTE_AT: usize = 3;
/// Palier d'appels d'outils d'un tour (Loki : 24 × 3 en mode Code).
pub const CALL_BUDGET: usize = 72;

/// Un heredoc d'une commande shell.
#[derive(Debug, PartialEq)]
struct Heredoc {
    /// Ligne qui ouvre le heredoc.
    header: usize,
    /// Lignes du contenu : `body.0..body.1` (la ligne de fin suit).
    body: (usize, usize),
    /// Fichier visé et écriture complète (`>`, `tee`) ou ajout (`>>`, `tee -a`).
    target: Option<(String, bool)>,
}

fn delim_end(c: char) -> bool {
    c.is_whitespace() || matches!(c, ';' | '&' | '|' | '<' | '>' | '(' | ')')
}

/// Délimiteurs des heredocs ouverts sur une ligne, avec `<<-` (tabulations).
fn delimiters(line: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let b = line.as_bytes();
    let mut i = 0;
    while let Some(p) = line[i..].find("<<") {
        let mut j = i + p + 2;
        if b.get(j) == Some(&b'<') {
            // Here-string `<<<` : pas un heredoc.
            i = j + 1;
            continue;
        }
        let strip = b.get(j) == Some(&b'-');
        if strip {
            j += 1;
        }
        while b.get(j).is_some_and(|c| *c == b' ' || *c == b'\t') {
            j += 1;
        }
        let rest = &line[j..];
        let (word, used) = match rest.chars().next() {
            Some(q @ ('\'' | '"')) => match rest[1..].find(q) {
                Some(end) => (rest[1..1 + end].to_string(), end + 2),
                None => (String::new(), rest.len()),
            },
            _ => {
                let end = rest.find(delim_end).unwrap_or(rest.len());
                (rest[..end].replace(['\\', '\'', '"'], ""), end)
            }
        };
        i = j + used.max(1).min(rest.len().max(1));
        if i > line.len() {
            i = line.len();
        }
        if !word.is_empty() {
            out.push((word, strip));
        }
        if i >= line.len() {
            break;
        }
    }
    out
}

fn unquote(s: &str) -> String {
    s.trim_matches(|c| c == '\'' || c == '"').to_string()
}

/// Mot shell en tête de `s` (guillemets simples ou doubles compris).
fn word_at(s: &str) -> &str {
    let s = s.trim_start();
    if let Some(q @ ('\'' | '"')) = s.chars().next()
        && let Some(end) = s[1..].find(q)
    {
        return &s[..end + 2];
    }
    &s[..s.find(delim_end).unwrap_or(s.len())]
}

/// Fichier écrit par la ligne d'en-tête : redirection `>`/`>>` (pas `2>`,
/// `>&`) ou `tee [-a] f`.
fn target(line: &str) -> Option<(String, bool)> {
    let b = line.as_bytes();
    let mut i = 0;
    while let Some(p) = line[i..].find('>') {
        let at = i + p;
        let append = b.get(at + 1) == Some(&b'>');
        let after = at + if append { 2 } else { 1 };
        let fd = at > 0 && b[at - 1].is_ascii_digit();
        let dup = b.get(after) == Some(&b'&');
        if !fd && !dup {
            let w = unquote(word_at(&line[after..]));
            if !w.is_empty() && w != "/dev/null" {
                return Some((w, !append));
            }
        }
        i = after;
        if i >= line.len() {
            break;
        }
    }
    let t = line.find("tee ")?;
    let rest = line[t + 4..].trim_start();
    let (append, rest) = match rest.strip_prefix("-a ") {
        Some(r) => (true, r),
        None => (false, rest),
    };
    let w = unquote(word_at(rest));
    (!w.is_empty()).then_some((w, !append))
}

fn heredocs(lines: &[&str]) -> Vec<Heredoc> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let delims = delimiters(lines[i]);
        if delims.is_empty() {
            i += 1;
            continue;
        }
        let header = i;
        let tgt = target(lines[header]);
        let mut j = header + 1;
        for (k, (d, strip)) in delims.iter().enumerate() {
            let start = j;
            while j < lines.len() {
                let l = if *strip {
                    lines[j].trim_start_matches('\t')
                } else {
                    lines[j]
                };
                if l.trim_end_matches('\r') == d {
                    break;
                }
                j += 1;
            }
            out.push(Heredoc {
                header,
                body: (start, j),
                target: if k == 0 { tgt.clone() } else { None },
            });
            j += 1;
        }
        i = j.max(header + 1);
    }
    out
}

/// Script shell d'un appel : `cmd`, `command` (chaîne) ou la plus longue
/// chaîne de `command` (`["bash", "-lc", "…"]`).
fn script(args: &Value) -> Option<&str> {
    let o = args.as_object()?;
    if let Some(s) = o.get("cmd").and_then(Value::as_str) {
        return Some(s);
    }
    match o.get("command")? {
        Value::String(s) => Some(s),
        Value::Array(a) => a.iter().filter_map(Value::as_str).max_by_key(|s| s.len()),
        _ => None,
    }
}

fn kib(n: usize) -> String {
    if n < 1024 {
        format!("{n} octets")
    } else {
        format!("{} Ko", n.div_ceil(1024))
    }
}

/// Arguments allégés d'un appel qui écrit au moins un gros heredoc, ou
/// `None` (rien à alléger, arguments illisibles).
pub fn shrink_write(args: &str) -> Option<String> {
    let v: Value = serde_json::from_str(args).ok()?;
    let s = script(&v)?;
    let lines: Vec<&str> = s.split('\n').collect();
    let docs = heredocs(&lines);
    let big = |h: &Heredoc| {
        let body = &lines[h.body.0..h.body.1];
        body.len() >= SHRINK_MIN_LINES || body.iter().map(|l| l.chars().count() + 1).sum::<usize>() >= SHRINK_MIN_CHARS
    };
    if !docs.iter().any(big) {
        return None;
    }
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    for h in docs.iter().filter(|h| big(h)) {
        out.extend(lines[i..h.body.0].iter().map(|l| l.to_string()));
        let body = &lines[h.body.0..h.body.1];
        let bytes: usize = body.iter().map(|l| l.len() + 1).sum();
        out.push(format!(
            "«{} lignes ({}) retirées du fil par EngineX»",
            body.len(),
            kib(bytes)
        ));
        i = h.body.1;
    }
    out.extend(lines[i..].iter().map(|l| l.to_string()));
    let resume = format!(
        "Commande déjà exécutée. EngineX a retiré du fil le contenu écrit ; le fichier est intact sur \
         le disque (relis-le par tranches avec sed -n si besoin, ne le réécris pas pour autant) :\n{}",
        out.join("\n")
    );
    Some(json!({ "enginex_resume": resume }).to_string())
}

/// Fichiers écrits en entier par un appel (`cat > f <<`, `tee f <<`).
pub fn full_writes(args: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<Value>(args) else {
        return Vec::new();
    };
    let Some(s) = script(&v) else {
        return Vec::new();
    };
    let lines: Vec<&str> = s.split('\n').collect();
    heredocs(&lines)
        .into_iter()
        .filter_map(|h| h.target)
        .filter(|(_, full)| *full)
        .map(|(p, _)| p)
        .collect()
}

/// Note de rappel à `calls` appels (palier exact), ou `None`.
pub fn budget_note(calls: usize) -> Option<String> {
    if calls == 0 || !calls.is_multiple_of(CALL_BUDGET) {
        return None;
    }
    Some(match calls / CALL_BUDGET {
        1 => format!(
            "[EngineX] {calls} appels d'outils dans ce seul tour. C'est beaucoup : arrête d'explorer et termine \
             ce qui reste, sauf s'il manque vraiment une étape précise."
        ),
        2 => format!(
            "[EngineX] {calls} appels d'outils : tu tournes en rond. Relire un fichier déjà lu ou réécrire un \
             fichier déjà écrit n'apporte rien. Termine maintenant avec ce que tu as et dis ce qui reste incertain."
        ),
        _ => {
            format!("[EngineX] {calls} appels d'outils. Arrête-toi : écris ta réponse finale sans autre appel d'outil.")
        }
    })
}

/// Note après la `n`-ième écriture complète de `path` dans le tour.
pub fn rewrite_note(path: &str, n: usize) -> Option<String> {
    (n >= REWRITE_NOTE_AT).then(|| {
        format!(
            "[EngineX] {path} vient d'être réécrit en entier pour la {n}e fois dans ce tour. Ne le réécris plus : \
             il est sur le disque. Vérifie-le par tranches (wc -l, grep -n, sed -n 'a,bp') et corrige seulement \
             ce qui manque, par une modification ciblée ou `cat >>`."
        )
    })
}

/// Notes du tour en cours : `call_id` → texte à ajouter au résultat.
/// `calls` : (call_id, arguments) dans l'ordre du tour.
pub fn turn_notes<'a>(calls: impl IntoIterator<Item = (&'a str, &'a str)>) -> HashMap<String, String> {
    let mut notes: HashMap<String, String> = HashMap::new();
    let mut writes: HashMap<String, usize> = HashMap::new();
    for (n, (id, args)) in calls.into_iter().enumerate() {
        let mut parts = Vec::new();
        for p in full_writes(args) {
            let c = writes.entry(p.clone()).or_insert(0);
            *c += 1;
            parts.extend(rewrite_note(&p, *c));
        }
        parts.extend(budget_note(n + 1));
        if !parts.is_empty() {
            notes.insert(id.to_string(), parts.join("\n"));
        }
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(s: &str) -> String {
        json!({ "cmd": s, "workdir": "/work" }).to_string()
    }

    fn body(n: usize) -> String {
        (1..=n).map(|i| format!("ligne {i}")).collect::<Vec<_>>().join("\n")
    }

    #[test]
    fn small_heredoc_is_kept() {
        assert_eq!(
            shrink_write(&cmd(&format!("cat > a.txt <<'EOF'\n{}\nEOF", body(5)))),
            None
        );
        assert_eq!(shrink_write(&cmd("ls -la")), None);
        assert_eq!(shrink_write("pas du json"), None);
    }

    #[test]
    fn big_heredoc_is_shrunk_and_rest_kept() {
        let s = format!(
            "mkdir -p web && cat > web/app.js <<'EOF'\n{}\nEOF\nnode --check web/app.js",
            body(400)
        );
        let out = shrink_write(&cmd(&s)).expect("allégé");
        let v: Value = serde_json::from_str(&out).unwrap();
        let r = v["enginex_resume"].as_str().unwrap();
        assert!(
            r.contains("mkdir -p web && cat > web/app.js <<'EOF'\n«400 lignes ("),
            "{r}"
        );
        assert!(
            r.ends_with("retirées du fil par EngineX»\nEOF\nnode --check web/app.js"),
            "{r}"
        );
        assert!(!r.contains("ligne 1\n"));
        assert!(v.get("cmd").is_none(), "plus de commande exécutable dans le résumé");
        // Déterministe.
        assert_eq!(shrink_write(&cmd(&s)).unwrap(), out);
    }

    #[test]
    fn long_lines_count_as_big() {
        let s = format!("cat > a.css <<EOF\n{}\nEOF", "x".repeat(2500));
        assert!(shrink_write(&cmd(&s)).is_some());
    }

    #[test]
    fn several_heredocs_and_forms() {
        let s = format!(
            "cat > a.html <<'EOF'\n{}\nEOF\ncat <<-\"FIN\" >> b.js\n\t{}\n\tFIN\necho ok",
            body(3),
            body(50).replace('\n', "\n\t")
        );
        let out = shrink_write(&cmd(&s)).unwrap();
        let r: Value = serde_json::from_str(&out).unwrap();
        let r = r["enginex_resume"].as_str().unwrap();
        assert!(
            r.contains("cat > a.html <<'EOF'\nligne 1\nligne 2\nligne 3\nEOF"),
            "petit gardé : {r}"
        );
        assert!(r.contains("<<-\"FIN\" >> b.js\n«50 lignes"), "{r}");
        assert!(r.ends_with("\n\tFIN\necho ok"), "{r}");
    }

    #[test]
    fn command_array_and_string() {
        let a = json!({"command": ["bash", "-lc", format!("cat > x.py <<'EOF'\n{}\nEOF", body(40))]}).to_string();
        assert!(shrink_write(&a).is_some());
        let b = json!({"command": format!("tee x.py <<EOF\n{}\nEOF", body(40))}).to_string();
        assert!(shrink_write(&b).is_some());
        assert_eq!(full_writes(&b), vec!["x.py"]);
    }

    #[test]
    fn here_string_and_unclosed() {
        assert_eq!(shrink_write(&cmd("grep x <<< \"$(cat big)\"")), None);
        // Heredoc sans fin : tout le reste est le contenu.
        let s = format!("cat > z.txt <<EOF\n{}", body(35));
        assert!(shrink_write(&cmd(&s)).unwrap().contains("«35 lignes"));
    }

    #[test]
    fn full_write_targets() {
        assert_eq!(
            full_writes(&cmd("cat > 'src/a b.js' <<'EOF'\nx\nEOF")),
            vec!["src/a b.js"]
        );
        assert_eq!(full_writes(&cmd("cat >src/a.js <<'EOF'\nx\nEOF")), vec!["src/a.js"]);
        assert_eq!(full_writes(&cmd("cat <<'EOF' > a.js 2>&1\nx\nEOF")), vec!["a.js"]);
        assert!(full_writes(&cmd("cat >> a.js <<'EOF'\nx\nEOF")).is_empty());
        assert!(full_writes(&cmd("tee -a a.js <<'EOF'\nx\nEOF")).is_empty());
        assert!(full_writes(&cmd("cat a.js > /dev/null")).is_empty(), "pas un heredoc");
    }

    #[test]
    fn notes_for_rewrites_and_budget() {
        let w = cmd("cat > app.js <<'EOF'\nx\nEOF");
        let other = cmd("ls");
        let mut calls: Vec<(String, String)> = Vec::new();
        for i in 0..3 {
            calls.push((format!("w{i}"), w.clone()));
            calls.push((format!("o{i}"), other.clone()));
        }
        let notes = turn_notes(calls.iter().map(|(a, b)| (a.as_str(), b.as_str())));
        assert_eq!(notes.len(), 1);
        assert!(notes["w2"].contains("app.js vient d'être réécrit en entier pour la 3e fois"));

        let many: Vec<(String, String)> = (0..150).map(|i| (format!("c{i}"), other.clone())).collect();
        let notes = turn_notes(many.iter().map(|(a, b)| (a.as_str(), b.as_str())));
        assert_eq!(notes.len(), 2);
        assert!(notes["c71"].contains("72 appels"));
        assert!(notes["c143"].contains("tournes en rond"));
        assert!(budget_note(216).unwrap().contains("Arrête-toi"));
        assert_eq!(budget_note(73), None);
    }
}
