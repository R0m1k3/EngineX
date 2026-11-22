//! Page Tester : lance des cas de codage fixes sur le preset actif, audite
//! ce que le modèle a fait, puis reçoit la note de Michael.
//!
//! Un run = une exécution d'un ou plusieurs cas, l'un après l'autre. Chaque cas
//! a sa session (origine `bench`, sans secrets du coffre, sans correction
//! automatique ni « Essai de l'app »). L'audit rejoue les critères du cas dans
//! la sandbox, relève des alertes et des métriques ; la note n'est acceptée
//! qu'après l'audit.

pub mod cases;

use std::{collections::BTreeMap, path::Path, time::Duration};

use axum::{
    Json,
    extract::{Path as UrlPath, State},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    bridge,
    error::{ApiError, ApiJson},
    presets, sessions, snapshots,
    state::{AppState, now},
    workspace,
};
use cases::Case;

const MAX_TURN_S: u64 = 1200;
const CRITERION_TIMEOUT_S: u64 = 240;
const BIG_FILE_LINES: usize = 400;
const REPEAT_COMMAND: usize = 3;
const PROOF_MAX: usize = 2000;

/// Verrou de démarrage : un seul run actif à la fois.
static START: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// La session appartient-elle à un cas Tester ? (sandbox sans secrets)
pub async fn is_bench_session(state: &AppState, session_id: &str) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM bench_cases WHERE session_id = ?)")
        .bind(session_id)
        .fetch_one(state.db())
        .await
        .unwrap_or(0)
        != 0
}

// ---- Notation ----

/// Note manuelle sur 100 : moyenne des 4 axes (1 à 5) ramenée à 0-100.
pub fn manual_score(axes: &[i64; 4]) -> f64 {
    let mean = axes.iter().sum::<i64>() as f64 / 4.0;
    (mean - 1.0) / 4.0 * 100.0
}

/// Note finale : moyenne de l'auto et de la manuelle, ou la manuelle seule si
/// aucun critère auto. `None` tant que la note manuelle manque.
pub fn final_score(auto: Option<f64>, manual: Option<f64>) -> Option<f64> {
    let m = manual?;
    Some(match auto {
        Some(a) => (a + m) / 2.0,
        None => m,
    })
}

// ---- Routes ----

pub async fn list_cases() -> Json<Value> {
    Json(json!(
        cases::all()
            .iter()
            .map(|c| json!({"key": c.key, "title": c.title, "prompt": c.prompt, "criteria_count": c.criteria.len(), "timeout_s": c.timeout_s}))
            .collect::<Vec<_>>()
    ))
}

#[derive(Deserialize)]
pub struct StartBody {
    cases: Option<Vec<String>>,
}

pub async fn start(State(state): State<AppState>, ApiJson(b): ApiJson<StartBody>) -> Result<Json<Value>, ApiError> {
    let keys: Vec<&'static Case> = match b.cases.filter(|c| !c.is_empty()) {
        Some(list) => list
            .iter()
            .map(|k| cases::get(k).ok_or_else(|| ApiError::Invalid(format!("cas inconnu : {k}"))))
            .collect::<Result<_, _>>()?,
        None => cases::all().iter().collect(),
    };
    let _g = START.lock().await;
    let active: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bench_runs WHERE status = 'running'")
        .fetch_one(state.db())
        .await?;
    if active > 0 {
        return Err(ApiError::JobRunning);
    }
    let preset = presets::active_id(&state).await?.ok_or(ApiError::NoActivePreset)?;
    if crate::v1::not_ready(&state.supervisor().state()).is_some() {
        return Err(ApiError::EngineNotReady);
    }
    let run_id: i64 = sqlx::query_scalar(
        "INSERT INTO bench_runs (preset_id, status, started_at) VALUES (?, 'running', ?) RETURNING id",
    )
    .bind(preset)
    .bind(now())
    .fetch_one(state.db())
    .await?;
    let mut ids = Vec::new();
    for c in &keys {
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO bench_cases (run_id, case_key, status) VALUES (?, ?, 'pending') RETURNING id",
        )
        .bind(run_id)
        .bind(c.key)
        .fetch_one(state.db())
        .await?;
        ids.push((id, *c));
    }
    let st = state.clone();
    tokio::spawn(async move { run_task(st, run_id, preset, ids).await });
    Ok(Json(json!({"id": run_id})))
}

pub async fn list_runs(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let rows: Vec<(i64, i64, String, i64, Option<i64>)> = sqlx::query_as(
        "SELECT id, preset_id, status, started_at, finished_at FROM bench_runs ORDER BY id DESC LIMIT 100",
    )
    .fetch_all(state.db())
    .await?;
    let mut out = Vec::new();
    for (id, preset, status, started, finished) in rows {
        let cs = case_rows(&state, id).await?;
        let scores: Vec<f64> = cs.iter().filter_map(|c| c["final"].as_f64()).collect();
        let avg = (!scores.is_empty()).then(|| scores.iter().sum::<f64>() / scores.len() as f64);
        out.push(json!({
            "id": id, "preset_id": preset, "preset": preset_name(&state, preset).await, "status": status,
            "started_at": started, "finished_at": finished, "cases": cs.len(),
            "rated": scores.len(), "average": avg,
        }));
    }
    Ok(Json(json!(out)))
}

pub async fn get_run(State(state): State<AppState>, UrlPath(id): UrlPath<i64>) -> Result<Json<Value>, ApiError> {
    let (preset, status, started, finished): (i64, String, i64, Option<i64>) =
        sqlx::query_as("SELECT preset_id, status, started_at, finished_at FROM bench_runs WHERE id = ?")
            .bind(id)
            .fetch_optional(state.db())
            .await?
            .ok_or(ApiError::NotFound)?;
    Ok(Json(json!({
        "id": id, "preset_id": preset, "preset": preset_name(&state, preset).await, "status": status,
        "started_at": started, "finished_at": finished, "cases": case_rows(&state, id).await?,
    })))
}

async fn preset_name(state: &AppState, id: i64) -> String {
    sqlx::query_scalar::<_, String>("SELECT name FROM presets WHERE id = ?")
        .bind(id)
        .fetch_optional(state.db())
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| format!("preset {id} (supprimé)"))
}

type CaseRow = (
    i64,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    Option<f64>,
    Option<String>,
    Option<f64>,
    Option<String>,
    Option<i64>,
);

async fn case_rows(state: &AppState, run_id: i64) -> Result<Vec<Value>, ApiError> {
    let rows: Vec<CaseRow> = sqlx::query_as(
        "SELECT id, case_key, session_id, status, reason, audit_json, auto_score, manual_json, manual_score, comment, audited_at \
         FROM bench_cases WHERE run_id = ? ORDER BY id",
    )
    .bind(run_id)
    .fetch_all(state.db())
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, key, sid, status, reason, audit, auto, manual, mscore, comment, audited)| {
                let title = cases::get(&key).map(|c| c.title).unwrap_or("(cas retiré)");
                json!({
                    "id": id, "key": key, "title": title, "session_id": sid, "status": status, "reason": reason,
                    "audit": audit.and_then(|a| serde_json::from_str::<Value>(&a).ok()),
                    "auto_score": auto, "manual": manual.and_then(|m| serde_json::from_str::<Value>(&m).ok()),
                    "manual_score": mscore, "final": final_score(auto, mscore), "comment": comment,
                    "audited_at": audited,
                })
            },
        )
        .collect())
}

#[derive(Deserialize)]
pub struct RateBody {
    fonctionne: i64,
    qualite: i64,
    rendu: i64,
    consigne: i64,
    comment: Option<String>,
}

pub async fn rate(
    State(state): State<AppState>,
    UrlPath((run_id, case_id)): UrlPath<(i64, i64)>,
    ApiJson(b): ApiJson<RateBody>,
) -> Result<Json<Value>, ApiError> {
    let axes = [b.fonctionne, b.qualite, b.rendu, b.consigne];
    if axes.iter().any(|v| !(1..=5).contains(v)) {
        return Err(ApiError::BadRequest);
    }
    let audited: Option<Option<i64>> =
        sqlx::query_scalar("SELECT audited_at FROM bench_cases WHERE id = ? AND run_id = ?")
            .bind(case_id)
            .bind(run_id)
            .fetch_optional(state.db())
            .await?;
    match audited {
        None => return Err(ApiError::NotFound),
        Some(None) => return Err(ApiError::NotAudited),
        Some(Some(_)) => {}
    }
    let manual = json!({"fonctionne": b.fonctionne, "qualite": b.qualite, "rendu": b.rendu, "consigne": b.consigne});
    let comment = b
        .comment
        .map(|c| c.trim().chars().take(4000).collect::<String>())
        .filter(|c| !c.is_empty());
    sqlx::query("UPDATE bench_cases SET manual_json = ?, manual_score = ?, comment = ? WHERE id = ?")
        .bind(manual.to_string())
        .bind(manual_score(&axes))
        .bind(comment)
        .bind(case_id)
        .execute(state.db())
        .await?;
    Ok(Json(json!({"ok": true, "manual_score": manual_score(&axes)})))
}

pub async fn stop(State(state): State<AppState>, UrlPath(id): UrlPath<i64>) -> Result<Json<Value>, ApiError> {
    let r =
        sqlx::query("UPDATE bench_runs SET status = 'stopped', finished_at = ? WHERE id = ? AND status = 'running'")
            .bind(now())
            .bind(id)
            .execute(state.db())
            .await?;
    if r.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    // Interrompt le tour en cours : la tâche de fond voit le statut `stopped`.
    let sid: Option<String> = sqlx::query_scalar(
        "SELECT session_id FROM bench_cases WHERE run_id = ? AND status = 'running' AND session_id IS NOT NULL",
    )
    .bind(id)
    .fetch_optional(state.db())
    .await?;
    if let Some(sid) = sid {
        let _ = bridge::hub(&state, &sid).interrupt().await;
    }
    Ok(Json(json!({"ok": true})))
}

pub async fn delete_run(State(state): State<AppState>, UrlPath(id): UrlPath<i64>) -> Result<Json<Value>, ApiError> {
    let status: String = sqlx::query_scalar("SELECT status FROM bench_runs WHERE id = ?")
        .bind(id)
        .fetch_optional(state.db())
        .await?
        .ok_or(ApiError::NotFound)?;
    if status == "running" {
        return Err(ApiError::JobRunning);
    }
    let sids: Vec<String> =
        sqlx::query_scalar("SELECT session_id FROM bench_cases WHERE run_id = ? AND session_id IS NOT NULL")
            .bind(id)
            .fetch_all(state.db())
            .await?;
    for sid in sids {
        let _ = sessions::delete(State(state.clone()), UrlPath(sid)).await;
    }
    sqlx::query("DELETE FROM bench_runs WHERE id = ?")
        .bind(id)
        .execute(state.db())
        .await?;
    Ok(Json(json!({"ok": true})))
}

/// (run, preset, cas, auto, manuelle)
type ScoreRow = (i64, i64, String, Option<f64>, Option<f64>);
/// preset -> cas -> (somme des notes, nombre, runs)
type Acc = BTreeMap<i64, BTreeMap<String, (f64, usize, std::collections::BTreeSet<i64>)>>;

/// Moyenne des notes finales par preset et par cas (cas notés seulement).
pub async fn compare(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let rows: Vec<ScoreRow> = sqlx::query_as(
        "SELECT r.id, r.preset_id, c.case_key, c.auto_score, c.manual_score \
         FROM bench_cases c JOIN bench_runs r ON r.id = c.run_id WHERE c.manual_score IS NOT NULL",
    )
    .fetch_all(state.db())
    .await?;
    // preset -> cas -> (somme, nombre, runs)
    let mut acc: Acc = BTreeMap::new();
    for (run, preset, key, auto, manual) in rows {
        let Some(f) = final_score(auto, manual) else { continue };
        let e = acc.entry(preset).or_default().entry(key).or_default();
        e.0 += f;
        e.1 += 1;
        e.2.insert(run);
    }
    let mut out = Vec::new();
    for (preset, per_case) in acc {
        let all_runs: std::collections::BTreeSet<i64> = per_case.values().flat_map(|v| v.2.iter().copied()).collect();
        let cases_json: Vec<Value> = per_case
            .iter()
            .map(|(k, (sum, n, runs))| json!({"key": k, "average": sum / *n as f64, "runs": runs.len()}))
            .collect();
        let total: f64 = per_case.values().map(|(s, n, _)| s / *n as f64).sum::<f64>() / per_case.len() as f64;
        out.push(json!({
            "preset_id": preset, "preset": preset_name(&state, preset).await,
            "runs": all_runs.len(), "average": total, "cases": cases_json,
        }));
    }
    Ok(Json(json!(out)))
}

// ---- Exécution ----

async fn run_status(state: &AppState, run_id: i64) -> String {
    sqlx::query_scalar::<_, String>("SELECT status FROM bench_runs WHERE id = ?")
        .bind(run_id)
        .fetch_optional(state.db())
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
}

async fn finish_run(state: &AppState, run_id: i64, status: &str) {
    let _ = sqlx::query("UPDATE bench_runs SET status = ?, finished_at = ? WHERE id = ? AND status = 'running'")
        .bind(status)
        .bind(now())
        .bind(run_id)
        .execute(state.db())
        .await;
}

async fn run_task(state: AppState, run_id: i64, preset: i64, cases_list: Vec<(i64, &'static Case)>) {
    let topic = match workspace::ensure_topic(&state, "bench", "bench").await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("tester : topic bench : {e:?}");
            finish_run(&state, run_id, "stopped").await;
            return;
        }
    };
    let total = cases_list.len();
    let mut stopped = false;
    for (case_id, case) in cases_list {
        if run_status(&state, run_id).await != "running" {
            stopped = true;
            break;
        }
        // Le preset actif a changé : le lot s'arrête (les cas restants restent en attente).
        if presets::active_id(&state).await.ok().flatten() != Some(preset) {
            tracing::warn!("tester : preset actif changé, run {run_id} arrêté");
            stopped = true;
            break;
        }
        run_case(&state, run_id, preset, case_id, case, &topic).await;
    }
    if stopped {
        finish_run(&state, run_id, "stopped").await;
    } else {
        finish_run(&state, run_id, "done").await;
    }
    crate::feed::emit(
        &state,
        crate::feed::Note::new("bench", "info", format!("Tester : run {run_id} terminé ({total} cas)")).link("#bench"),
    );
}

async fn fail_case(state: &AppState, case_id: i64, reason: &str) {
    let _ = sqlx::query("UPDATE bench_cases SET status = 'failed', reason = ?, finished_at = ? WHERE id = ?")
        .bind(reason)
        .bind(now())
        .bind(case_id)
        .execute(state.db())
        .await;
}

fn write_file(dir: &Path, rel: &str, content: &str) -> std::io::Result<()> {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(p, content)
}

async fn run_case(
    state: &AppState,
    run_id: i64,
    preset: i64,
    case_id: i64,
    case: &'static Case,
    topic: &workspace::TopicInfo,
) {
    let (sid, _, work_dir) = match sessions::create_session(state, topic, Some(preset), "auto", false).await {
        Ok(x) => x,
        Err(e) => {
            fail_case(state, case_id, &format!("session : {e}")).await;
            return;
        }
    };
    let Some(work_dir) = work_dir else {
        fail_case(state, case_id, "pas de dossier propre pour la session").await;
        return;
    };
    let dir = snapshots::own_dir(&topic.dir, &work_dir);
    for (rel, content) in case.files {
        if let Err(e) = write_file(&dir, rel, content) {
            fail_case(state, case_id, &format!("fichier de départ {rel} : {e}")).await;
            return;
        }
    }
    let _ = sqlx::query("UPDATE chat_sessions SET title = ? WHERE id = ?")
        .bind(format!("Tester : {}", case.title))
        .bind(&sid)
        .execute(state.db())
        .await;
    // La ligne du cas est écrite avant le tour : la sandbox démarre sans secrets.
    let started = now();
    let _ = sqlx::query("UPDATE bench_cases SET session_id = ?, status = 'running', started_at = ? WHERE id = ?")
        .bind(&sid)
        .bind(started)
        .bind(case_id)
        .execute(state.db())
        .await;
    let timeout = Duration::from_secs(case.timeout_s.min(MAX_TURN_S));
    let (outcome, message) = crate::crons::run_prompt(state, &sid, case.prompt, timeout, "bench").await;
    let stopped = run_status(state, run_id).await != "running";
    let (status, reason) = if stopped {
        ("failed", Some("arrêté".to_string()))
    } else if outcome == "ok" {
        ("done", None)
    } else if message.starts_with("délai dépassé") {
        ("failed", Some("plafond de temps".to_string()))
    } else {
        ("failed", Some(message.chars().take(300).collect::<String>()))
    };
    let audit = audit(state, case, &sid, &dir, started).await;
    let auto: Option<f64> = {
        let crit = audit["criteria"].as_array().cloned().unwrap_or_default();
        (!crit.is_empty())
            .then(|| crit.iter().filter(|c| c["ok"].as_bool() == Some(true)).count() as f64 / crit.len() as f64 * 100.0)
    };
    let _ = sqlx::query(
        "UPDATE bench_cases SET status = ?, reason = ?, audit_json = ?, auto_score = ?, audited_at = ?, finished_at = ? \
         WHERE id = ?",
    )
    .bind(status)
    .bind(reason)
    .bind(audit.to_string())
    .bind(auto)
    .bind(now())
    .bind(now())
    .bind(case_id)
    .execute(state.db())
    .await;
}

// ---- Audit ----

async fn audit(state: &AppState, case: &Case, sid: &str, dir: &Path, since: i64) -> Value {
    // Les fichiers de test fournis sont remis tels quels avant les critères.
    for rel in case.protected {
        if let Some((_, content)) = case.files.iter().find(|(r, _)| r == rel) {
            let _ = write_file(dir, rel, content);
        }
    }
    let criteria = run_criteria(state, case, sid).await;
    json!({
        "criteria": criteria,
        "auto_count": criteria.len(),
        "alerts": alerts(state, case, sid, dir).await,
        "metrics": metrics(state, sid, since).await,
    })
}

async fn run_criteria(state: &AppState, case: &Case, sid: &str) -> Vec<Value> {
    let hub = bridge::hub(state, sid);
    let _ = crate::sessions::make_room(state, sid).await;
    let conn = hub.ready(state).await;
    let mut out = Vec::new();
    for c in case.criteria {
        let Ok((client, cwd)) = &conn else {
            out.push(json!({"name": c.name, "ok": false, "proof": format!("sandbox : {}", conn.as_ref().err().cloned().unwrap_or_default())}));
            continue;
        };
        let script = format!("export BENCH_DIR=\"$PWD\"\n{}", c.script);
        let mut params = enginex_codex::types::command_exec(
            &["bash", "-lc", &script],
            cwd,
            None,
            None,
            Some(CRITERION_TIMEOUT_S * 1000),
        );
        params["outputBytesCap"] = json!(64 * 1024);
        let r = client
            .request_with_timeout(
                "command/exec",
                params,
                Some(Duration::from_secs(CRITERION_TIMEOUT_S + 30)),
            )
            .await;
        hub.touch_activity();
        let (ok, proof) = match r {
            Ok(v) => {
                let code = v["exitCode"].as_i64().unwrap_or(-1);
                let text = format!(
                    "{}{}",
                    v["stdout"].as_str().unwrap_or(""),
                    v["stderr"].as_str().unwrap_or("")
                );
                (code == 0, format!("code {code}\n{}", crate::crons::tail(&text)))
            }
            Err(enginex_codex::Error::Timeout(_)) => (false, "délai dépassé".to_string()),
            Err(e) => (false, format!("command/exec : {e}")),
        };
        let proof = state.masker().mask(&proof).into_owned();
        out.push(json!({"name": c.name, "ok": ok, "proof": proof.chars().take(PROOF_MAX).collect::<String>()}));
    }
    out
}

/// Commandes lancées par le modèle (`commandExecution`), dans l'ordre.
async fn commands(state: &AppState, sid: &str) -> Vec<String> {
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT data FROM chat_items WHERE session_id = ? AND kind = 'commandExecution' ORDER BY seq",
    )
    .bind(sid)
    .fetch_all(state.db())
    .await
    .unwrap_or_default();
    rows.iter()
        .filter_map(|d| serde_json::from_str::<Value>(d).ok())
        .filter_map(|v| v["command"].as_str().map(str::to_string))
        .collect()
}

fn normalize(cmd: &str) -> String {
    cmd.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Alertes d'affichage (jamais dans le score) : fichier trop long, même
/// commande répétée, paquet ajouté que la consigne ne cite pas.
async fn alerts(state: &AppState, case: &Case, sid: &str, dir: &Path) -> Vec<Value> {
    let mut out = Vec::new();
    let mut big = Vec::new();
    big_files(dir, dir, 0, case, &mut big);
    for (rel, n) in big {
        out.push(json!({"kind": "big_file", "text": format!("{rel} : {n} lignes (> {BIG_FILE_LINES})")}));
    }
    let cmds = commands(state, sid).await;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for c in &cmds {
        *counts.entry(normalize(c)).or_default() += 1;
    }
    for (c, n) in counts {
        if n >= REPEAT_COMMAND {
            out.push(json!({"kind": "repeated_command", "text": format!("{n} fois : {}", c.chars().take(160).collect::<String>())}));
        }
    }
    let prompt = case.prompt.to_lowercase();
    for c in &cmds {
        for pkg in added_packages(c) {
            if !prompt.contains(&pkg.to_lowercase()) {
                out.push(json!({"kind": "added_package", "text": format!("{pkg} ajouté ({})", c.chars().take(120).collect::<String>())}));
            }
        }
    }
    out.dedup();
    out
}

/// Paquets installés par une commande (`npm i x`, `pip install x`, `cargo add x`…).
fn added_packages(cmd: &str) -> Vec<String> {
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
            // `pip install -r x`, `npm install` seul, options et chemins : pas des paquets.
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
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            if matches!(
                name.as_str(),
                ".git" | "node_modules" | "__pycache__" | ".enginex" | "target" | "venv" | ".venv"
            ) {
                continue;
            }
            big_files(root, &p, depth + 1, case, out);
        } else if let Ok(text) = std::fs::read_to_string(&p) {
            let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy().to_string();
            if case.files.iter().any(|(r, _)| *r == rel) || rel.ends_with(".json") || rel.ends_with(".lock") {
                continue;
            }
            let n = text.lines().count();
            if n > BIG_FILE_LINES {
                out.push((rel, n));
            }
        }
    }
}

/// Durée du tour (hors attente en file : de `started_at` à `completed_at`),
/// jetons, débit moyen, nombre de commandes et de fichiers écrits.
async fn metrics(state: &AppState, sid: &str, since: i64) -> Value {
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
    let items: Vec<(String, String)> =
        sqlx::query_as("SELECT kind, data FROM chat_items WHERE session_id = ? ORDER BY seq")
            .bind(sid)
            .fetch_all(state.db())
            .await
            .unwrap_or_default();
    let (mut cmds, mut files, mut tps_sum, mut tps_n) = (0, 0, 0.0, 0);
    for (kind, data) in &items {
        match kind.as_str() {
            "commandExecution" => cmds += 1,
            "fileChange" => files += 1,
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
        "duration_s": duration, "tokens": tokens, "commands": cmds, "file_changes": files,
        "tps": (tps_n > 0).then(|| tps_sum / tps_n as f64), "turns": turns.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_scale() {
        assert_eq!(manual_score(&[1, 1, 1, 1]), 0.0);
        assert_eq!(manual_score(&[5, 5, 5, 5]), 100.0);
        assert_eq!(manual_score(&[3, 3, 3, 3]), 50.0);
    }

    #[test]
    fn final_is_half_and_half() {
        assert_eq!(final_score(Some(100.0), Some(50.0)), Some(75.0));
        assert_eq!(final_score(None, Some(50.0)), Some(50.0));
        assert_eq!(final_score(Some(100.0), None), None);
    }

    #[test]
    fn packages_are_found() {
        assert_eq!(added_packages("npm install jsdom express"), vec!["jsdom", "express"]);
        assert_eq!(added_packages("cd x && pip install requests"), vec!["requests"]);
        assert!(added_packages("npm install").is_empty());
        assert!(added_packages("pip install -r requirements.txt").is_empty());
        assert!(added_packages("python3 -m unittest").is_empty());
    }

    #[test]
    fn every_case_is_well_formed() {
        for c in cases::all() {
            assert!(c.criteria.len() >= 3, "{} : au moins 3 critères", c.key);
            assert!(c.timeout_s <= MAX_TURN_S, "{}", c.key);
            for p in c.protected {
                assert!(c.files.iter().any(|(r, _)| r == p), "{} : {p} non fourni", c.key);
            }
        }
        assert_eq!(cases::all().len(), 10);
    }
}
