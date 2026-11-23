//! Page Tester : lance 15 tests fixes sur le preset actif et les contrôle.
//!
//! Un run = un ou plusieurs tests, l'un après l'autre, une session chacun
//! (origine `bench` : sans secrets du coffre, sans mémoire, sans correction
//! automatique ni « Essai de l'app »). L'audit (`audit.rs`) rejoue les
//! contrôles : shell dans la sandbox, scénarios dans le Chromium d'EngineX,
//! règles sur le journal. Résultat = points gagnés / points du test, verdict
//! Réussi / Partiel / Échoué. Les tests visuels reçoivent en plus la note
//! visuelle de Michael (1 à 5), jamais mélangée aux points.

mod audit;
pub mod cases;
pub mod journal;

use std::{collections::BTreeMap, path::Path, time::Duration};

use axum::{
    Json,
    extract::{Path as UrlPath, State},
    http::header,
    response::IntoResponse,
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
use cases::{Case, Family};

const MAX_TURN_S: u64 = 1200;

/// Verrou de démarrage : un seul run actif à la fois.
static START: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// La session appartient-elle à un test ? (sandbox sans secrets, sans mémoire)
pub async fn is_bench_session(state: &AppState, session_id: &str) -> bool {
    sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM bench_cases WHERE session_id = ?)")
        .bind(session_id)
        .fetch_one(state.db())
        .await
        .unwrap_or(0)
        != 0
}

// ---- Routes ----

pub async fn list_cases() -> Json<Value> {
    Json(json!(
        cases::all()
            .iter()
            .map(|c| json!({
                "key": c.key, "title": c.title, "family": c.family.key(), "summary": c.summary,
                "prompt": c.prompt, "timeout_s": c.timeout_s, "points_max": c.points_max(),
                "checks": c.checks.iter().map(|k| json!({"name": k.name, "points": k.points, "essential": k.essential})).collect::<Vec<_>>(),
            }))
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
            .map(|k| cases::get(k).ok_or_else(|| ApiError::Invalid(format!("test inconnu : {k}"))))
            .collect::<Result<_, _>>()?,
        None => cases::all().to_vec(),
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
            "INSERT INTO bench_cases (run_id, case_key, status, points_max) VALUES (?, ?, 'pending', ?) RETURNING id",
        )
        .bind(run_id)
        .bind(c.key)
        .bind(c.points_max())
        .fetch_one(state.db())
        .await?;
        ids.push((id, *c));
    }
    let st = state.clone();
    tokio::spawn(async move { run_task(st, run_id, preset, ids).await });
    Ok(Json(json!({"id": run_id})))
}

/// Résumé d'un ensemble de tests : réussis, résultat en %, note visuelle moyenne.
pub fn summarize(cs: &[Value]) -> Value {
    let audited: Vec<&Value> = cs.iter().filter(|c| c["verdict"].is_string()).collect();
    let passed = audited.iter().filter(|c| c["verdict"] == "passed").count();
    let partial = audited.iter().filter(|c| c["verdict"] == "partial").count();
    let (pts, max): (f64, f64) = audited.iter().fold((0.0, 0.0), |(p, m), c| {
        (
            p + c["points"].as_f64().unwrap_or(0.0),
            m + c["points_max"].as_f64().unwrap_or(0.0),
        )
    });
    let visuals: Vec<f64> = cs.iter().filter_map(|c| c["visual"].as_f64()).collect();
    let to_rate = cs
        .iter()
        .filter(|c| c["family"] == "visual" && c["verdict"].is_string() && c["visual"].is_null())
        .count();
    let duration: i64 = cs
        .iter()
        .filter_map(|c| c["audit"]["metrics"]["duration_s"].as_i64())
        .sum();
    let tps: Vec<f64> = cs
        .iter()
        .filter_map(|c| c["audit"]["metrics"]["tps"].as_f64())
        .collect();
    json!({
        "cases": cs.len(), "audited": audited.len(), "passed": passed, "partial": partial,
        "failed": audited.len() - passed - partial,
        "result": (max > 0.0).then(|| pts / max * 100.0),
        "visual": (!visuals.is_empty()).then(|| visuals.iter().sum::<f64>() / visuals.len() as f64),
        "visual_rated": visuals.len(), "to_rate": to_rate, "duration_s": duration,
        "tps": (!tps.is_empty()).then(|| tps.iter().sum::<f64>() / tps.len() as f64),
    })
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
        out.push(json!({
            "id": id, "preset_id": preset, "preset": preset_name(&state, preset).await, "status": status,
            "started_at": started, "finished_at": finished, "summary": summarize(&cs),
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
    let cs = case_rows(&state, id).await?;
    Ok(Json(json!({
        "id": id, "preset_id": preset, "preset": preset_name(&state, preset).await, "status": status,
        "started_at": started, "finished_at": finished, "summary": summarize(&cs), "cases": cs,
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

#[derive(sqlx::FromRow)]
struct CaseRow {
    id: i64,
    case_key: String,
    session_id: Option<String>,
    status: String,
    reason: Option<String>,
    audit_json: Option<String>,
    points: Option<f64>,
    points_max: Option<i64>,
    verdict: Option<String>,
    visual: Option<i64>,
    comment: Option<String>,
    audited_at: Option<i64>,
    started_at: Option<i64>,
    finished_at: Option<i64>,
}

fn case_json(r: CaseRow) -> Value {
    let def = cases::get(&r.case_key);
    json!({
        "id": r.id, "key": r.case_key, "title": def.map(|c| c.title).unwrap_or("(test retiré)"),
        "family": def.map(|c| c.family.key()).unwrap_or("code"),
        "session_id": r.session_id, "status": r.status, "reason": r.reason,
        "audit": r.audit_json.and_then(|a| serde_json::from_str::<Value>(&a).ok()),
        "points": r.points, "points_max": r.points_max.or(def.map(|c| c.points_max() as i64)),
        "verdict": r.verdict, "visual": r.visual, "comment": r.comment,
        "audited_at": r.audited_at, "started_at": r.started_at, "finished_at": r.finished_at,
    })
}

async fn case_rows(state: &AppState, run_id: i64) -> Result<Vec<Value>, ApiError> {
    let rows: Vec<CaseRow> = sqlx::query_as(
        "SELECT id, case_key, session_id, status, reason, audit_json, points, points_max, verdict, visual, comment, \
         audited_at, started_at, finished_at FROM bench_cases WHERE run_id = ? ORDER BY id",
    )
    .bind(run_id)
    .fetch_all(state.db())
    .await?;
    Ok(rows.into_iter().map(case_json).collect())
}

#[derive(Deserialize)]
pub struct RateBody {
    visual: Option<i64>,
    comment: Option<String>,
}

/// Note visuelle (tests visuels, 1 à 5) et commentaire (tous les tests).
pub async fn rate(
    State(state): State<AppState>,
    UrlPath((run_id, case_id)): UrlPath<(i64, i64)>,
    ApiJson(b): ApiJson<RateBody>,
) -> Result<Json<Value>, ApiError> {
    let row: Option<(String, Option<i64>)> =
        sqlx::query_as("SELECT case_key, audited_at FROM bench_cases WHERE id = ? AND run_id = ?")
            .bind(case_id)
            .bind(run_id)
            .fetch_optional(state.db())
            .await?;
    let Some((key, audited)) = row else {
        return Err(ApiError::NotFound);
    };
    if audited.is_none() {
        return Err(ApiError::NotAudited);
    }
    let visual_case = cases::get(&key).is_some_and(|c| c.family == Family::Visual);
    if let Some(v) = b.visual
        && (!visual_case || !(1..=5).contains(&v))
    {
        return Err(ApiError::BadRequest);
    }
    let comment = b
        .comment
        .map(|c| c.trim().chars().take(4000).collect::<String>())
        .filter(|c| !c.is_empty());
    sqlx::query("UPDATE bench_cases SET visual = COALESCE(?, visual), comment = ? WHERE id = ?")
        .bind(b.visual)
        .bind(comment)
        .bind(case_id)
        .execute(state.db())
        .await?;
    Ok(Json(json!({"ok": true})))
}

/// Capture d'un test (`desktop` ou `phone`).
pub async fn shot(
    State(state): State<AppState>,
    UrlPath((case_id, name)): UrlPath<(i64, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let name = name.trim_end_matches(".png");
    if !matches!(name, "desktop" | "phone") {
        return Err(ApiError::NotFound);
    }
    let path = audit::shots_dir(&state, case_id).join(format!("{name}.png"));
    let bytes = tokio::fs::read(path).await.map_err(|_| ApiError::NotFound)?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "private, max-age=86400"),
        ],
        bytes,
    ))
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
    close_pending(&state, id).await;
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
    let rows: Vec<(i64, Option<String>)> = sqlx::query_as("SELECT id, session_id FROM bench_cases WHERE run_id = ?")
        .bind(id)
        .fetch_all(state.db())
        .await?;
    for (cid, sid) in rows {
        let _ = std::fs::remove_dir_all(audit::shots_dir(&state, cid));
        if let Some(sid) = sid
            && let Err(e) = sessions::delete(State(state.clone()), UrlPath(sid.clone())).await
        {
            tracing::warn!("tester : suppression de la session {sid} : {e:?}");
        }
    }
    sqlx::query("DELETE FROM bench_runs WHERE id = ?")
        .bind(id)
        .execute(state.db())
        .await?;
    Ok(Json(json!({"ok": true})))
}

/// Par preset : résumé de tous ses tests audités, et résultat moyen par test.
pub async fn compare(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let runs: Vec<(i64, i64)> = sqlx::query_as("SELECT id, preset_id FROM bench_runs ORDER BY id")
        .fetch_all(state.db())
        .await?;
    let mut per_preset: BTreeMap<i64, (Vec<Value>, usize)> = BTreeMap::new();
    for (run, preset) in runs {
        let cs: Vec<Value> = case_rows(&state, run)
            .await?
            .into_iter()
            .filter(|c| c["verdict"].is_string())
            .collect();
        if cs.is_empty() {
            continue;
        }
        let e = per_preset.entry(preset).or_default();
        e.0.extend(cs);
        e.1 += 1;
    }
    let mut out = Vec::new();
    for (preset, (cs, nruns)) in per_preset {
        let mut by_case: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
        for c in &cs {
            by_case
                .entry(c["key"].as_str().unwrap_or("").to_string())
                .or_default()
                .push(c);
        }
        let cases_json: Vec<Value> = by_case
            .iter()
            .map(|(k, v)| {
                let pct: Vec<f64> = v
                    .iter()
                    .filter_map(|c| Some(c["points"].as_f64()? / c["points_max"].as_f64().filter(|m| *m > 0.0)?))
                    .collect();
                let vis: Vec<f64> = v.iter().filter_map(|c| c["visual"].as_f64()).collect();
                json!({
                    "key": k, "samples": v.len(),
                    "result": (!pct.is_empty()).then(|| pct.iter().sum::<f64>() / pct.len() as f64 * 100.0),
                    "passed": v.iter().filter(|c| c["verdict"] == "passed").count(),
                    "visual": (!vis.is_empty()).then(|| vis.iter().sum::<f64>() / vis.len() as f64),
                })
            })
            .collect();
        out.push(json!({
            "preset_id": preset, "preset": preset_name(&state, preset).await, "runs": nruns,
            "summary": summarize(&cs), "cases": cases_json,
        }));
    }
    Ok(Json(json!(out)))
}

// ---- Exécution ----

async fn run_status(state: &AppState, run_id: i64) -> String {
    sqlx::query_scalar("SELECT status FROM bench_runs WHERE id = ?")
        .bind(run_id)
        .fetch_optional(state.db())
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "stopped".into())
}

/// Les tests jamais lancés d'un run arrêté sont clos (pas d'attente sans fin).
async fn close_pending(state: &AppState, run_id: i64) {
    let _ = sqlx::query(
        "UPDATE bench_cases SET status = 'failed', reason = 'arrêté avant de commencer' WHERE run_id = ? AND status = 'pending'",
    )
    .bind(run_id)
    .execute(state.db())
    .await;
}

async fn finish_run(state: &AppState, run_id: i64, status: &str) {
    let _ = sqlx::query(
        "UPDATE bench_runs SET status = ?, finished_at = COALESCE(finished_at, ?) WHERE id = ? AND status = 'running'",
    )
    .bind(status)
    .bind(now())
    .bind(run_id)
    .execute(state.db())
    .await;
    if status != "done" {
        close_pending(state, run_id).await;
    }
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
        // Le preset actif a changé : le lot s'arrête (les tests restants sont clos).
        if presets::active_id(&state).await.ok().flatten() != Some(preset) {
            tracing::warn!("tester : preset actif changé, run {run_id} arrêté");
            stopped = true;
            break;
        }
        run_case(&state, run_id, preset, case_id, case, &topic).await;
    }
    finish_run(&state, run_id, if stopped { "stopped" } else { "done" }).await;
    let word = if run_status(&state, run_id).await == "stopped" {
        "arrêté"
    } else {
        "terminé"
    };
    crate::feed::emit(
        &state,
        crate::feed::Note::new("bench", "info", format!("Tester : run {run_id} {word} ({total} tests)")).link("#bench"),
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

async fn set_reason(state: &AppState, case_id: i64, reason: &str) {
    let _ = sqlx::query("UPDATE bench_cases SET reason = ? WHERE id = ?")
        .bind((!reason.is_empty()).then_some(reason))
        .bind(case_id)
        .execute(state.db())
        .await;
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
    for (rel, content) in case.start_files() {
        if let Err(e) = write_file(&dir, &rel, &content) {
            fail_case(state, case_id, &format!("fichier de départ {rel} : {e}")).await;
            return;
        }
    }
    let _ = sqlx::query("UPDATE chat_sessions SET title = ? WHERE id = ?")
        .bind(format!("Tester : {}", case.title))
        .bind(&sid)
        .execute(state.db())
        .await;
    // La ligne du test est écrite avant toute sandbox : elle démarre sans secrets.
    let started = now();
    let _ = sqlx::query("UPDATE bench_cases SET session_id = ?, status = 'running', started_at = ? WHERE id = ?")
        .bind(&sid)
        .bind(started)
        .bind(case_id)
        .execute(state.db())
        .await;
    if run_status(state, run_id).await != "running" {
        fail_case(state, case_id, "arrêté").await;
        return;
    }
    if let Some(setup) = case.setup {
        set_reason(state, case_id, "préparation…").await;
        match audit::exec(state, &sid, setup, 120).await {
            Ok((0, _)) => set_reason(state, case_id, "").await,
            Ok((code, out)) => {
                fail_case(
                    state,
                    case_id,
                    &format!("préparation (code {code}) : {}", crate::crons::tail(&out)),
                )
                .await;
                return;
            }
            Err(e) => {
                fail_case(state, case_id, &format!("préparation : {e}")).await;
                return;
            }
        }
    }
    let timeout = Duration::from_secs(case.timeout_s.min(MAX_TURN_S));
    let (outcome, message) = crate::crons::run_prompt(state, &sid, case.prompt, timeout, "bench").await;
    let (status, reason) = if outcome == "ok" {
        ("done", None)
    } else if message.starts_with("délai dépassé") {
        (
            "failed",
            Some("plafond de temps atteint (contrôlé quand même)".to_string()),
        )
    } else {
        ("failed", Some(message.chars().take(300).collect::<String>()))
    };
    // Arrêté : pas d'audit (il lancerait encore les contrôles).
    if run_status(state, run_id).await != "running" {
        fail_case(state, case_id, "arrêté").await;
        return;
    }
    set_reason(state, case_id, "contrôles en cours…").await;
    let audit = audit::run(state, case, case_id, &sid, &dir, started).await;
    let _ = sqlx::query(
        "UPDATE bench_cases SET status = ?, reason = ?, audit_json = ?, points = ?, points_max = ?, verdict = ?, \
         audited_at = ?, finished_at = ? WHERE id = ?",
    )
    .bind(status)
    .bind(reason)
    .bind(audit.to_string())
    .bind(audit["points"].as_f64())
    .bind(audit["points_max"].as_i64())
    .bind(audit["verdict"].as_str())
    .bind(now())
    .bind(now())
    .bind(case_id)
    .execute(state.db())
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_counts() {
        let c = |v: &str, p: f64, fam: &str, vis: Option<i64>| json!({"verdict": v, "points": p, "points_max": 20, "family": fam, "visual": vis});
        let s = summarize(&[
            c("passed", 20.0, "visual", Some(4)),
            c("partial", 10.0, "visual", None),
            c("failed", 0.0, "code", None),
            json!({"verdict": null, "family": "code"}),
        ]);
        assert_eq!(s["passed"], 1);
        assert_eq!(s["partial"], 1);
        assert_eq!(s["failed"], 1);
        assert_eq!(s["result"].as_f64(), Some(50.0));
        assert_eq!(s["visual"].as_f64(), Some(4.0));
        assert_eq!(s["to_rate"], 1);
    }
}
