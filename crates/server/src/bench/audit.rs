//! Audit d'un test : contrôles shell (sandbox), navigateur (Chromium d'EngineX),
//! journal ; captures bureau/téléphone ; alertes et métriques d'affichage.

use std::{collections::BTreeMap, path::Path, sync::Arc, time::Duration};

use serde_json::{Value, json};

use super::{
    cases::{Case, Check, Family, Kind},
    journal::Journal,
};
use crate::{appgate, bridge, state::AppState};

const SHELL_TIMEOUT_S: u64 = 240;
const PROOF_MAX: usize = 2000;
const BIG_FILE_LINES: usize = 400;
const REPEAT_COMMAND: usize = 3;
const MAX_SCAN_BYTES: u64 = 1 << 20;
/// Port du petit serveur de fichiers lancé dans la sandbox pour les contrôles navigateur.
pub const HIDDEN_DIR: &str = ".bench-hidden";
const HELPERS_JS: &str = include_str!("data/helpers.js");
const BENCHH_PY: &str = include_str!("data/hidden/benchh.py");

pub const DESKTOP: (u32, u32) = (1280, 800);
pub const PHONE: (u32, u32) = (390, 844);
/// Hauteur maximale d'une capture pleine page.
const SHOT_MAX_H: u32 = 2400;

/// `bash -lc` dans la sandbox de la session : (code, sortie).
pub async fn exec(state: &AppState, sid: &str, script: &str, timeout_s: u64) -> Result<(i64, String), String> {
    let hub = bridge::hub(state, sid);
    let _ = crate::sessions::make_room(state, sid).await;
    let (client, cwd) = hub.ready(state).await.map_err(|e| format!("sandbox : {e}"))?;
    let script = format!("export BENCH_DIR=\"$PWD\" BENCH_HIDDEN=\"$PWD/{HIDDEN_DIR}\"\n{script}");
    let mut params =
        enginex_codex::types::command_exec(&["bash", "-lc", &script], &cwd, None, None, Some(timeout_s * 1000));
    params["outputBytesCap"] = json!(64 * 1024);
    let r = client
        .request_with_timeout("command/exec", params, Some(Duration::from_secs(timeout_s + 30)))
        .await;
    hub.touch_activity();
    match r {
        Ok(v) => Ok((
            v["exitCode"].as_i64().unwrap_or(-1),
            format!(
                "{}{}",
                v["stdout"].as_str().unwrap_or(""),
                v["stderr"].as_str().unwrap_or("")
            ),
        )),
        Err(enginex_codex::Error::Timeout(_)) => Err("délai dépassé".into()),
        Err(e) => Err(format!("command/exec : {e}")),
    }
}

/// Fichiers fournis modifiés par le modèle (avant leur remise).
fn changed_protected(case: &Case, files: &[(String, String)], dir: &Path) -> Vec<String> {
    case.protected
        .iter()
        .filter(|rel| {
            let want = files.iter().find(|(r, _)| r == *rel).map(|(_, c)| c.as_bytes());
            let got = crate::files::read_beneath(dir, rel, 64 << 20);
            want.is_some() && got.as_deref() != want
        })
        .map(|r| r.to_string())
        .collect()
}

/// Le dossier est écrit par la sandbox : `files::*_beneath` ne suit aucun lien.
fn restore_protected(case: &Case, files: &[(String, String)], dir: &Path) {
    for rel in case.protected {
        if let Some((_, content)) = files.iter().find(|(r, _)| r == rel) {
            let res = crate::files::remove_beneath(dir, rel)
                .and_then(|()| crate::files::create_beneath(dir, rel, content.as_bytes()));
            if let Err(e) = res {
                tracing::warn!("tester : restauration de {rel} : {e}");
            }
        }
    }
}

/// Un contrôle terminé, pour la fiche.
fn result(c: &Check, ok: bool, earned: f64, proof: String, kind: &str) -> Value {
    let proof: String = proof.chars().take(PROOF_MAX).collect();
    json!({
        "name": c.name, "points": c.points, "earned": (earned * 10.0).round() / 10.0,
        "essential": c.essential, "ok": ok, "proof": proof, "kind": kind,
    })
}

/// Points d'un contrôle shell : `BENCH_SCORE a/b` → au prorata, sinon tout ou rien.
pub fn shell_points(points: u32, ok: bool, out: &str) -> f64 {
    let score = out.lines().rev().find_map(|l| {
        let (a, b) = l.trim().strip_prefix("BENCH_SCORE ")?.split_once('/')?;
        Some((a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?))
    });
    match score {
        Some((a, b)) if b > 0.0 => {
            if ok {
                points as f64
            } else {
                points as f64 * (a / b).clamp(0.0, 1.0)
            }
        }
        _ if ok => points as f64,
        _ => 0.0,
    }
}

/// Verdict : un contrôle essentiel raté → échoué ; tout bon → réussi ; sinon partiel.
pub fn verdict(checks: &[Value]) -> &'static str {
    let ok = |c: &Value| c["ok"].as_bool() == Some(true);
    if checks.iter().any(|c| c["essential"].as_bool() == Some(true) && !ok(c)) {
        "failed"
    } else if checks.iter().all(ok) {
        "passed"
    } else {
        "partial"
    }
}

pub async fn run(state: &AppState, case: &Case, case_id: i64, sid: &str, dir: &Path, since: i64) -> Value {
    let files = case.start_files();
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT kind, data FROM chat_items WHERE session_id = ? ORDER BY seq")
            .bind(sid)
            .fetch_all(state.db())
            .await
            .unwrap_or_default();
    let mut journal = Journal::from_rows(&rows);
    journal.protected_changed = changed_protected(case, &files, dir);
    restore_protected(case, &files, dir);
    let hidden_ok = write_hidden(state, case, sid, dir).await;

    let mut out: Vec<Value> = Vec::new();
    let mut console: Vec<String> = Vec::new();
    let mut page: Option<Result<PageCtx, String>> = None;
    let mut console_slot = None;
    for c in case.checks {
        let v = match &c.kind {
            Kind::Shell(script) => {
                if !hidden_ok.is_empty() && script.contains("BENCH_HIDDEN") {
                    result(c, false, 0.0, hidden_ok.clone(), "shell")
                } else {
                    match exec(state, sid, script, SHELL_TIMEOUT_S).await {
                        Ok((code, text)) => {
                            let ok = code == 0;
                            let pts = shell_points(c.points, ok, &text);
                            let proof = state
                                .masker()
                                .mask(&format!("code {code}\n{}", crate::crons::tail(&text)))
                                .into_owned();
                            result(c, ok, pts, proof, "shell")
                        }
                        Err(e) => result(c, false, 0.0, e, "shell"),
                    }
                }
            }
            Kind::Page { path, phone, steps } => {
                if page.is_none() {
                    page = Some(PageCtx::open(state, sid, case_id).await);
                }
                match page.as_ref().unwrap() {
                    Ok(ctx) => {
                        let (ok, proof, lines) = ctx.check(path, *phone, steps).await;
                        console.extend(lines);
                        result(c, ok, if ok { c.points as f64 } else { 0.0 }, proof, "page")
                    }
                    Err(e) => result(c, false, 0.0, format!("page impossible à ouvrir : {e}"), "page"),
                }
            }
            Kind::ConsoleClean => {
                console_slot = Some(out.len());
                json!(null)
            }
            Kind::Log(rule) => match rule(&journal) {
                Ok(p) => result(c, true, c.points as f64, p, "log"),
                Err(p) => result(c, false, 0.0, p, "log"),
            },
        };
        out.push(v);
    }
    if let Some(i) = console_slot {
        let c = &case.checks[i];
        out[i] = match &page {
            Some(Ok(_)) if console.is_empty() => result(c, true, c.points as f64, "aucune erreur".into(), "console"),
            Some(Ok(_)) => {
                let text = enginex_browser::format_console(&console);
                result(c, false, 0.0, state.masker().mask(&text).into_owned(), "console")
            }
            _ => result(c, false, 0.0, "la page n'a pas pu être ouverte".into(), "console"),
        };
    }
    // Captures pour la note visuelle (et pour voir ce que le test a produit).
    let mut shots = Vec::new();
    if let Some(p) = case.page {
        if page.is_none() {
            page = Some(PageCtx::open(state, sid, case_id).await);
        }
        if let Some(Ok(ctx)) = &page {
            let shots_dir = shots_dir(state, case_id);
            for (name, phone) in [("desktop", false), ("phone", true)] {
                match ctx.shot(p, phone).await {
                    Ok(png) => {
                        if std::fs::create_dir_all(&shots_dir).is_ok()
                            && std::fs::write(shots_dir.join(format!("{name}.png")), png).is_ok()
                        {
                            shots.push(name);
                        }
                    }
                    Err(e) => tracing::warn!("tester : capture {name} : {e}"),
                }
            }
        }
    }
    if let Some(Ok(ctx)) = &page {
        ctx.close(state).await;
    }
    let _ = exec(state, sid, &format!("rm -rf {HIDDEN_DIR}"), 30).await;

    let points: f64 = out.iter().filter_map(|c| c["earned"].as_f64()).sum();
    json!({
        "checks": out,
        "points": (points * 10.0).round() / 10.0,
        "points_max": case.points_max(),
        "verdict": verdict(&out),
        "shots": shots,
        "visual": case.family == Family::Visual,
        "alerts": alerts(case, &journal_commands(&rows), dir),
        "metrics": metrics(state, sid, since, &rows).await,
    })
}

pub fn shots_dir(state: &AppState, case_id: i64) -> std::path::PathBuf {
    state.config().data_dir.join("bench").join(case_id.to_string())
}

/// Tests cachés : `.bench-hidden/` vidé dans la sandbox puis écrit sans suivre de lien.
/// Rend un message d'erreur (vide si tout va bien).
async fn write_hidden(state: &AppState, case: &Case, sid: &str, dir: &Path) -> String {
    if case.hidden.is_empty() {
        return String::new();
    }
    if let Err(e) = exec(state, sid, &format!("rm -rf {HIDDEN_DIR}"), 30).await {
        return format!("tests cachés : {e}");
    }
    let mut all: Vec<(&str, &str)> = case.hidden.to_vec();
    all.push(("benchh.py", BENCHH_PY));
    for (name, content) in all {
        if let Err(e) = crate::files::create_beneath(dir, &format!("{HIDDEN_DIR}/{name}"), content.as_bytes()) {
            return format!("tests cachés : écriture de {name} : {e}");
        }
    }
    String::new()
}

// ---- Navigateur ----

struct PageCtx {
    tab: Arc<enginex_browser::Session>,
    origin: String,
    key: String,
}

impl PageCtx {
    /// Pages statiques servies par EngineX depuis le dossier de la session
    /// (sas `preview::FILES`) : ni serveur à lancer dans la sandbox, ni relais.
    async fn open(state: &AppState, sid: &str, case_id: i64) -> Result<PageCtx, String> {
        let origin = appgate::open_app(state, sid, crate::preview::FILES).await?;
        let key = format!("bench-audit-{case_id}");
        let tab = state.0.browser.get(&key).await?;
        let _ = tab.call("Log.enable", None).await;
        Ok(PageCtx { tab, origin, key })
    }

    async fn close(&self, state: &AppState) {
        state.0.browser.close(&self.key).await;
    }

    async fn viewport(&self, phone: bool) -> Result<(), String> {
        let (w, h) = if phone { PHONE } else { DESKTOP };
        self.tab
            .call(
                "Emulation.setDeviceMetricsOverride",
                Some(json!({"width": w, "height": h, "deviceScaleFactor": 1, "mobile": phone})),
            )
            .await
            .map(|_| ())
    }

    /// Page propre (stockage vidé), à la taille voulue.
    async fn load(&self, path: &str, phone: bool) -> Result<String, String> {
        self.viewport(phone).await?;
        let url = format!("{}/{}", self.origin, path.trim_start_matches('/'));
        self.tab.navigate(&url).await?;
        let _ = self
            .tab
            .eval("try{localStorage.clear();sessionStorage.clear()}catch(e){};1")
            .await;
        self.tab.take_console();
        self.tab.navigate(&url).await?;
        tokio::time::sleep(Duration::from_millis(400)).await;
        Ok(url)
    }

    async fn wait_ready(&self) {
        tokio::time::sleep(Duration::from_millis(700)).await;
        for _ in 0..50 {
            if let Ok(Some(v)) = self.tab.eval("document.readyState").await
                && v == "complete"
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    /// (ok, preuve, erreurs de console vues pendant le contrôle).
    async fn check(&self, path: &str, phone: bool, steps: &[&str]) -> (bool, String, Vec<String>) {
        let url = match self.load(path, phone).await {
            Ok(u) => u,
            Err(e) => return (false, format!("chargement : {e}"), self.tab.take_console()),
        };
        if phone {
            let w = self
                .tab
                .eval("innerWidth")
                .await
                .ok()
                .flatten()
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            if w > 500.0 {
                return (
                    false,
                    format!("pas de balise viewport : sur téléphone la page s'affiche en largeur bureau ({w} px)"),
                    self.tab.take_console(),
                );
            }
        }
        for (i, step) in steps.iter().enumerate() {
            match *step {
                "@reload" => {
                    if let Err(e) = self.tab.navigate(&url).await {
                        return (false, format!("rechargement : {e}"), self.tab.take_console());
                    }
                    tokio::time::sleep(Duration::from_millis(400)).await;
                }
                "@wait" => self.wait_ready().await,
                js => {
                    let expr = wrap_step(js);
                    let r = self.tab.eval(&expr).await;
                    let text = match r {
                        Ok(Some(Value::String(s))) => s,
                        Ok(v) => format!("KO: réponse inattendue {v:?}"),
                        Err(e) => format!("KO: {e}"),
                    };
                    if text != "OK" {
                        let msg = text.strip_prefix("KO: ").unwrap_or(&text).to_string();
                        let at = if steps.len() > 1 {
                            format!(" (étape {})", i + 1)
                        } else {
                            String::new()
                        };
                        return (false, format!("{msg}{at}"), self.tab.take_console());
                    }
                }
            }
        }
        (true, "scénario réussi".into(), self.tab.take_console())
    }

    async fn shot(&self, path: &str, phone: bool) -> Result<Vec<u8>, String> {
        self.load(path, phone).await?;
        tokio::time::sleep(Duration::from_millis(600)).await;
        let (w, h) = if phone { PHONE } else { DESKTOP };
        let full = self
            .tab
            .eval("Math.max(document.documentElement.scrollHeight, document.body ? document.body.scrollHeight : 0)")
            .await
            .ok()
            .flatten()
            .and_then(|v| v.as_f64())
            .unwrap_or(h as f64);
        let height = (full as u32).clamp(h, SHOT_MAX_H);
        let res = self
            .tab
            .call(
                "Page.captureScreenshot",
                Some(json!({
                    "format": "png", "captureBeyondViewport": true,
                    "clip": {"x": 0, "y": 0, "width": w, "height": height, "scale": 1},
                })),
            )
            .await?;
        let _ = self.tab.call("Emulation.clearDeviceMetricsOverride", None).await;
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD
            .decode(res["data"].as_str().unwrap_or(""))
            .map_err(|e| format!("capture illisible : {e}"))
    }
}

/// Une étape JS : aides + corps, « OK » ou « KO: raison ».
pub fn wrap_step(js: &str) -> String {
    format!("(async()=>{{\n{HELPERS_JS}\n{js}\n}})().then(()=>'OK',e=>'KO: '+(e&&e.message?e.message:String(e)))")
}

// ---- Alertes et métriques (affichage seulement) ----

fn journal_commands(rows: &[(String, String)]) -> Vec<String> {
    rows.iter()
        .filter(|(k, _)| k == "commandExecution")
        .filter_map(|(_, d)| serde_json::from_str::<Value>(d).ok())
        .filter_map(|v| v["command"].as_str().map(str::to_string))
        .collect()
}

fn alerts(case: &Case, cmds: &[String], dir: &Path) -> Vec<Value> {
    let mut out = Vec::new();
    let mut big = Vec::new();
    big_files(dir, dir, 0, case, &mut big);
    for (rel, n) in big {
        out.push(json!({"kind": "big_file", "text": format!("{rel} : {n} lignes (> {BIG_FILE_LINES})")}));
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for c in cmds {
        *counts
            .entry(c.split_whitespace().collect::<Vec<_>>().join(" "))
            .or_default() += 1;
    }
    for (c, n) in counts {
        if n >= REPEAT_COMMAND {
            out.push(json!({"kind": "repeated_command", "text": format!("{n} fois : {}", c.chars().take(160).collect::<String>())}));
        }
    }
    let prompt = case.prompt.to_lowercase();
    for c in cmds {
        for pkg in added_packages(c) {
            if !prompt.contains(&pkg.to_lowercase()) {
                out.push(json!({"kind": "added_package", "text": format!("{pkg} ajouté ({})", c.chars().take(120).collect::<String>())}));
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|a| seen.insert(a.to_string()));
    out
}

/// Paquets installés par une commande (`npm i x`, `pip install x`, `cargo add x`…).
pub fn added_packages(cmd: &str) -> Vec<String> {
    let mut found = Vec::new();
    for part in cmd.split(['&', ';', '|', '\n']) {
        let t: Vec<&str> = part.split_whitespace().collect();
        let pos = t
            .iter()
            .position(|w| matches!(*w, "npm" | "pnpm" | "yarn" | "pip" | "pip3" | "cargo"));
        let Some(i) = pos else { continue };
        let verb_ok = t.get(i + 1).is_some_and(|v| matches!(*v, "install" | "i" | "add"));
        if !verb_ok {
            continue;
        }
        for w in &t[i + 2..] {
            if w.starts_with('-') || w.contains('/') && !w.starts_with('@') || w.contains('.') && !w.starts_with('@') {
                continue;
            }
            found.push(w.to_string());
        }
    }
    found
}

fn big_files(root: &Path, dir: &Path, depth: usize, case: &Case, out: &mut Vec<(String, usize)>) {
    if depth > 4 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let given = case.start_files();
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            if matches!(
                name.as_str(),
                ".git" | "node_modules" | "__pycache__" | ".enginex" | "target" | "venv" | ".venv" | HIDDEN_DIR
            ) {
                continue;
            }
            big_files(root, &p, depth + 1, case, out);
        } else if ft.is_file() && e.metadata().is_ok_and(|m| m.len() <= MAX_SCAN_BYTES) {
            let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().to_string();
            if given.iter().any(|(r, _)| *r == rel)
                || rel.ends_with(".json")
                || rel.ends_with(".lock")
                || rel.ends_with(".log")
            {
                continue;
            }
            let Some(text) =
                crate::files::read_beneath(root, &rel, MAX_SCAN_BYTES as usize).and_then(|b| String::from_utf8(b).ok())
            else {
                continue;
            };
            let n = text.lines().count();
            if n > BIG_FILE_LINES {
                out.push((rel, n));
            }
        }
    }
}

/// Durée du tour (hors attente en file), jetons, débit moyen, commandes, outils.
async fn metrics(state: &AppState, sid: &str, since: i64, rows: &[(String, String)]) -> Value {
    let turns: Vec<(Option<i64>, i64, Option<String>)> = sqlx::query_as(
        "SELECT completed_at, started_at, usage FROM chat_turns WHERE session_id = ? AND started_at >= ? ORDER BY started_at",
    )
    .bind(sid)
    .bind(since)
    .fetch_all(state.db())
    .await
    .unwrap_or_default();
    let duration: i64 = turns.iter().map(|(end, start, _)| end.unwrap_or(*start) - start).sum();
    let tokens: i64 = turns
        .iter()
        .filter_map(|(_, _, u)| u.as_deref().and_then(|u| serde_json::from_str::<Value>(u).ok()))
        .map(|u| {
            u["total"]["totalTokens"]
                .as_i64()
                .or_else(|| u["totalTokens"].as_i64())
                .unwrap_or(0)
        })
        .sum();
    let (mut cmds, mut tools, mut tps_sum, mut tps_n) = (0, 0, 0.0, 0);
    for (kind, data) in rows {
        match kind.as_str() {
            "commandExecution" => cmds += 1,
            "mcpToolCall" => tools += 1,
            _ => {}
        }
        if let Some(t) = serde_json::from_str::<Value>(data)
            .ok()
            .and_then(|v| v["gen"]["tps"].as_f64())
            && t > 0.0
        {
            tps_sum += t;
            tps_n += 1;
        }
    }
    json!({
        "duration_s": duration, "tokens": tokens, "commands": cmds, "tools": tools,
        "tps": (tps_n > 0).then(|| tps_sum / tps_n as f64), "turns": turns.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_points_prorata() {
        assert_eq!(shell_points(10, true, "x"), 10.0);
        assert_eq!(shell_points(10, false, "x"), 0.0);
        assert_eq!(shell_points(10, false, "KO a\nBENCH_SCORE 7/10\n"), 7.0);
        assert_eq!(shell_points(6, true, "BENCH_SCORE 300/300"), 6.0);
    }

    #[test]
    fn verdicts() {
        let c = |ok: bool, ess: bool| json!({"ok": ok, "essential": ess});
        assert_eq!(verdict(&[c(true, true), c(true, false)]), "passed");
        assert_eq!(verdict(&[c(true, true), c(false, false)]), "partial");
        assert_eq!(verdict(&[c(false, true), c(true, false)]), "failed");
    }

    #[test]
    fn packages_are_found() {
        assert_eq!(added_packages("npm install express"), vec!["express"]);
        assert_eq!(added_packages("pip install -r requirements.txt"), Vec::<String>::new());
        assert_eq!(
            added_packages("cd x && npm i @scope/pkg lodash"),
            vec!["@scope/pkg", "lodash"]
        );
    }

    #[test]
    fn steps_are_wrapped() {
        let s = wrap_step("ok(1, 'x');");
        assert!(s.starts_with("(async()=>{") && s.contains("const ok =") && s.contains("ok(1, 'x');"));
    }
}
