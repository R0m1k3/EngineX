//! Tâches planifiées : une consigne Codex (`prompt`) ou une commande
//! (`shell`) lancée dans un topic, selon une expression cron (5 champs, avec
//! fuseau) ou toutes les N minutes.
//!
//! Chaque cron a sa session « Cron : <nom> » dans le topic, gardée d'une
//! exécution à l'autre : la consigne y est un tour normal, la commande passe
//! par `command/exec` dans sa sandbox (réseau et clés comme une session).
//! Une exécution à la fois par cron ; après un arrêt d'EngineX, une seule
//! exécution de rattrapage.

use std::{collections::HashSet, sync::Mutex, time::Duration};

use axum::{
    Json,
    extract::{Path as UrlPath, State},
};
use chrono::TimeZone;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    bridge,
    error::{ApiError, ApiJson},
    sessions,
    state::{AppState, now},
    workspace,
};

const OUTPUT_MAX: usize = 16 * 1024;
const RUNS_KEPT: i64 = 50;
const NAME_MAX: usize = 80;
const BODY_MAX: usize = 20_000;

/// Crons en cours d'exécution.
#[derive(Default)]
pub struct Running(Mutex<HashSet<i64>>);

impl Running {
    fn take(&self, id: i64) -> bool {
        self.0.lock().expect("crons").insert(id)
    }
    fn release(&self, id: i64) {
        self.0.lock().expect("crons").remove(&id);
    }
    pub fn contains(&self, id: i64) -> bool {
        self.0.lock().expect("crons").contains(&id)
    }
}

// ---- Planification ----

pub enum Schedule {
    Cron(Box<croner::Cron>),
    Every(i64),
}

/// `every:<minutes>` (1 à 10080) ou une expression cron à 5 champs.
pub fn parse_schedule(s: &str) -> Result<Schedule, String> {
    let s = s.trim();
    if let Some(m) = s.strip_prefix("every:") {
        let m: i64 = m.trim().parse().map_err(|_| "every:<minutes> attendu".to_string())?;
        if !(1..=10080).contains(&m) {
            return Err("intervalle de 1 minute à 7 jours".into());
        }
        return Ok(Schedule::Every(m));
    }
    if s.split_whitespace().count() != 5 {
        return Err("expression cron à 5 champs attendue (minute heure jour mois jour-de-semaine)".into());
    }
    s.parse::<croner::Cron>()
        .map(|c| Schedule::Cron(Box::new(c)))
        .map_err(|e| format!("expression cron invalide : {e}"))
}

pub fn parse_tz(tz: &str) -> Result<chrono_tz::Tz, String> {
    tz.trim().parse().map_err(|_| format!("fuseau inconnu : {tz}"))
}

/// Prochaine exécution strictement après `after` (secondes Unix).
pub fn next_after(s: &Schedule, tz: chrono_tz::Tz, after: i64) -> Option<i64> {
    match s {
        Schedule::Every(m) => Some(after + m * 60),
        Schedule::Cron(c) => {
            let t = tz.timestamp_opt(after, 0).single()?;
            c.find_next_occurrence(&t, false).ok().map(|d| d.timestamp())
        }
    }
}

// ---- Base ----

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct CronRow {
    pub id: i64,
    pub name: String,
    pub topic_id: i64,
    pub kind: String,
    pub body: String,
    pub schedule: String,
    pub tz: String,
    pub timeout_s: i64,
    pub enabled: bool,
    pub session_id: Option<String>,
    pub next_run: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    /// Preset du cron ; `None` = preset par défaut.
    pub preset_id: Option<i64>,
}

async fn row(state: &AppState, id: i64) -> Result<CronRow, ApiError> {
    sqlx::query_as("SELECT * FROM crons WHERE id = ?")
        .bind(id)
        .fetch_optional(state.db())
        .await?
        .ok_or(ApiError::NotFound)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CronBody {
    name: String,
    topic_id: i64,
    kind: String,
    body: String,
    schedule: String,
    #[serde(default = "default_tz")]
    tz: String,
    #[serde(default = "default_timeout")]
    timeout_s: i64,
    #[serde(default = "yes")]
    enabled: bool,
    #[serde(default)]
    preset_id: Option<i64>,
}

fn default_tz() -> String {
    "Europe/Paris".into()
}
fn default_timeout() -> i64 {
    1800
}
fn yes() -> bool {
    true
}

/// Réglages contrôlés, et la prochaine exécution s'il est actif.
fn check(b: &CronBody) -> Result<Option<i64>, String> {
    let name = b.name.trim();
    if name.is_empty() || name.chars().count() > NAME_MAX || name.contains('\n') {
        return Err(format!("nom vide ou trop long ({NAME_MAX} caractères)"));
    }
    if !matches!(b.kind.as_str(), "prompt" | "shell") {
        return Err("type : prompt ou shell".into());
    }
    if b.body.trim().is_empty() || b.body.chars().count() > BODY_MAX {
        return Err(format!("contenu vide ou trop long ({BODY_MAX} caractères)"));
    }
    if !(10..=7200).contains(&b.timeout_s) {
        return Err("délai de 10 s à 2 h".into());
    }
    let s = parse_schedule(&b.schedule)?;
    let tz = parse_tz(&b.tz)?;
    Ok(if b.enabled { next_after(&s, tz, now()) } else { None })
}

fn bad(m: String) -> ApiError {
    ApiError::Invalid(m)
}

// ---- API ----

pub async fn list(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let rows: Vec<CronRow> = sqlx::query_as("SELECT * FROM crons ORDER BY name COLLATE NOCASE")
        .fetch_all(state.db())
        .await?;
    let mut out = Vec::new();
    for r in rows {
        let last: Option<(String, i64, Option<i64>)> = sqlx::query_as(
            "SELECT status, started_at, ended_at FROM cron_runs WHERE cron_id = ? ORDER BY id DESC LIMIT 1",
        )
        .bind(r.id)
        .fetch_optional(state.db())
        .await?;
        let topic = workspace::topic(&state, r.topic_id).await.ok();
        out.push(json!({
            "id": r.id, "name": r.name, "topic_id": r.topic_id,
            "topic": topic.map(|t| format!("{} / {}", t.project_slug, t.slug)),
            "kind": r.kind, "body": r.body, "schedule": r.schedule, "tz": r.tz,
            "timeout_s": r.timeout_s, "enabled": r.enabled, "session_id": r.session_id,
            "next_run": r.next_run, "running": state.0.crons.contains(r.id), "preset_id": r.preset_id,
            "last": last.map(|(s, a, e)| json!({"status": s, "started_at": a, "ended_at": e})),
        }));
    }
    Ok(Json(Value::Array(out)))
}

pub async fn create(State(state): State<AppState>, ApiJson(b): ApiJson<CronBody>) -> Result<Json<Value>, ApiError> {
    let next = check(&b).map_err(bad)?;
    workspace::topic(&state, b.topic_id).await?;
    sessions::check_preset(&state, b.preset_id).await?;
    let t = now();
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO crons (name, topic_id, kind, body, schedule, tz, timeout_s, enabled, next_run, created_at, updated_at, \
         preset_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(b.name.trim())
    .bind(b.topic_id)
    .bind(&b.kind)
    .bind(&b.body)
    .bind(b.schedule.trim())
    .bind(b.tz.trim())
    .bind(b.timeout_s)
    .bind(b.enabled)
    .bind(next)
    .bind(t)
    .bind(t)
    .bind(b.preset_id)
    .fetch_one(state.db())
    .await?;
    Ok(Json(json!({"id": id, "next_run": next})))
}

pub async fn update(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<i64>,
    ApiJson(b): ApiJson<CronBody>,
) -> Result<Json<Value>, ApiError> {
    let old = row(&state, id).await?;
    let next = check(&b).map_err(bad)?;
    workspace::topic(&state, b.topic_id).await?;
    sessions::check_preset(&state, b.preset_id).await?;
    // Autre topic : nouvelle session au prochain passage.
    let session = (old.topic_id == b.topic_id).then_some(old.session_id).flatten();
    sqlx::query(
        "UPDATE crons SET name = ?, topic_id = ?, kind = ?, body = ?, schedule = ?, tz = ?, timeout_s = ?, \
         enabled = ?, next_run = ?, session_id = ?, updated_at = ?, preset_id = ? WHERE id = ?",
    )
    .bind(b.name.trim())
    .bind(b.topic_id)
    .bind(&b.kind)
    .bind(&b.body)
    .bind(b.schedule.trim())
    .bind(b.tz.trim())
    .bind(b.timeout_s)
    .bind(b.enabled)
    .bind(next)
    .bind(session)
    .bind(now())
    .bind(b.preset_id)
    .bind(id)
    .execute(state.db())
    .await?;
    Ok(Json(json!({"id": id, "next_run": next})))
}

pub async fn delete(State(state): State<AppState>, UrlPath(id): UrlPath<i64>) -> Result<Json<Value>, ApiError> {
    row(&state, id).await?;
    sqlx::query("DELETE FROM crons WHERE id = ?")
        .bind(id)
        .execute(state.db())
        .await?;
    Ok(Json(json!({"ok": true})))
}

pub async fn run_now(State(state): State<AppState>, UrlPath(id): UrlPath<i64>) -> Result<Json<Value>, ApiError> {
    let r = row(&state, id).await?;
    if !state.0.crons.take(id) {
        return Err(ApiError::Busy);
    }
    tokio::spawn(run_taken(state.clone(), r, "manual"));
    Ok(Json(json!({"started": true})))
}

pub async fn runs(State(state): State<AppState>, UrlPath(id): UrlPath<i64>) -> Result<Json<Vec<RunRow>>, ApiError> {
    row(&state, id).await?;
    let rows: Vec<RunRow> = sqlx::query_as(
        "SELECT id, trigger, status, started_at, ended_at, output, session_id FROM cron_runs \
         WHERE cron_id = ? ORDER BY id DESC LIMIT ?",
    )
    .bind(id)
    .bind(RUNS_KEPT)
    .fetch_all(state.db())
    .await?;
    Ok(Json(rows))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct RunRow {
    id: i64,
    trigger: String,
    status: String,
    started_at: i64,
    ended_at: Option<i64>,
    output: String,
    session_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewBody {
    schedule: String,
    #[serde(default = "default_tz")]
    tz: String,
}

/// Les 3 prochaines exécutions d'une planification (formulaire de l'UI).
pub async fn preview(ApiJson(b): ApiJson<PreviewBody>) -> Result<Json<Value>, ApiError> {
    let s = parse_schedule(&b.schedule).map_err(bad)?;
    let tz = parse_tz(&b.tz).map_err(bad)?;
    let mut at = now();
    let mut next = Vec::new();
    for _ in 0..3 {
        let Some(n) = next_after(&s, tz, at) else { break };
        next.push(n);
        at = n;
    }
    Ok(Json(json!({ "next": next })))
}

// ---- Planificateur ----

pub fn spawn(state: &AppState) {
    let state = state.clone();
    tokio::spawn(async move {
        let tick = state.config().timings.cron_tick;
        loop {
            if let Err(e) = tick_once(&state).await {
                tracing::warn!("crons : {e}");
            }
            tokio::time::sleep(tick).await;
        }
    });
}

/// Lance les crons échus. La prochaine date part de maintenant : une seule
/// exécution de rattrapage après un arrêt.
pub async fn tick_once(state: &AppState) -> Result<(), sqlx::Error> {
    let t = now();
    let due: Vec<CronRow> =
        sqlx::query_as("SELECT * FROM crons WHERE enabled = 1 AND next_run IS NOT NULL AND next_run <= ?")
            .bind(t)
            .fetch_all(state.db())
            .await?;
    for c in due {
        let next = match (parse_schedule(&c.schedule), parse_tz(&c.tz)) {
            (Ok(s), Ok(tz)) => next_after(&s, tz, t),
            _ => None,
        };
        sqlx::query("UPDATE crons SET next_run = ? WHERE id = ?")
            .bind(next)
            .bind(c.id)
            .execute(state.db())
            .await?;
        tokio::spawn(run(state.clone(), c, "schedule"));
    }
    Ok(())
}

// ---- Exécution ----

async fn insert_run(state: &AppState, cron: i64, trigger: &str, status: &str, output: &str) -> Option<i64> {
    let id = sqlx::query_scalar(
        "INSERT INTO cron_runs (cron_id, trigger, status, started_at, ended_at, output) \
         VALUES (?, ?, ?, ?, CASE WHEN ? = 'running' THEN NULL ELSE ? END, ?) RETURNING id",
    )
    .bind(cron)
    .bind(trigger)
    .bind(status)
    .bind(now())
    .bind(status)
    .bind(now())
    .bind(output)
    .fetch_one(state.db())
    .await
    .ok();
    // Historique borné.
    let _ = sqlx::query(
        "DELETE FROM cron_runs WHERE cron_id = ? AND id NOT IN \
         (SELECT id FROM cron_runs WHERE cron_id = ? ORDER BY id DESC LIMIT ?)",
    )
    .bind(cron)
    .bind(cron)
    .bind(RUNS_KEPT)
    .execute(state.db())
    .await;
    id
}

/// Une exécution du cron `c`.
pub async fn run(state: AppState, c: CronRow, trigger: &'static str) {
    if !state.0.crons.take(c.id) {
        insert_run(
            &state,
            c.id,
            trigger,
            "skipped",
            "la précédente exécution tourne encore",
        )
        .await;
        return;
    }
    run_taken(state, c, trigger).await;
}

/// Exécution, la place du cron déjà prise.
async fn run_taken(state: AppState, c: CronRow, trigger: &'static str) {
    let Some(run_id) = insert_run(&state, c.id, trigger, "running", "").await else {
        state.0.crons.release(c.id);
        return;
    };
    let (status, output, sid) = match session_for(&state, &c).await {
        Err(e) => ("failed", format!("session du cron : {}", reason(&e)), None),
        Ok(sid) => {
            let _ = sqlx::query("UPDATE cron_runs SET session_id = ? WHERE id = ?")
                .bind(&sid)
                .bind(run_id)
                .execute(state.db())
                .await;
            let timeout = Duration::from_secs(c.timeout_s.max(10) as u64);
            let (st, out) = if c.kind == "shell" {
                run_shell(&state, &sid, &c.body, timeout).await
            } else {
                run_prompt(&state, &sid, &c.body, timeout).await
            };
            (st, out, Some(sid))
        }
    };
    let output = tail(&state.masker().mask(&output));
    let _ = sqlx::query("UPDATE cron_runs SET status = ?, ended_at = ?, output = ?, session_id = ? WHERE id = ?")
        .bind(status)
        .bind(now())
        .bind(&output)
        .bind(&sid)
        .bind(run_id)
        .execute(state.db())
        .await;
    state.0.crons.release(c.id);
    tracing::info!("cron « {} » ({trigger}) : {status}", c.name);
    let (level, what) = if status == "ok" {
        ("success", "réussi")
    } else {
        ("error", "en échec")
    };
    crate::feed::emit(
        &state,
        crate::feed::Note::new("cron", level, format!("Cron « {} » {what}", c.name))
            .body(
                output
                    .chars()
                    .rev()
                    .take(600)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<String>(),
            )
            .link("#crons")
            .important(status != "ok"),
    );
}

/// Session du cron, recréée si elle a disparu.
async fn session_for(state: &AppState, c: &CronRow) -> Result<String, ApiError> {
    if let Some(sid) = &c.session_id
        && sessions::row(state, sid).await.is_ok()
    {
        sqlx::query("UPDATE chat_sessions SET preset_id = ? WHERE id = ?")
            .bind(c.preset_id)
            .bind(sid)
            .execute(state.db())
            .await?;
        return Ok(sid.clone());
    }
    workspace::topic(state, c.topic_id).await?;
    let id = uuid::Uuid::new_v4().to_string();
    let t = now();
    sqlx::query(
        "INSERT INTO chat_sessions (id, topic_id, title, approval, created_at, updated_at, preset_id) \
         VALUES (?, ?, ?, 'auto', ?, ?, ?)",
    )
    .bind(&id)
    .bind(c.topic_id)
    .bind(format!("Cron : {}", c.name))
    .bind(t)
    .bind(t)
    .bind(c.preset_id)
    .execute(state.db())
    .await?;
    sqlx::query("UPDATE crons SET session_id = ? WHERE id = ?")
        .bind(&id)
        .bind(c.id)
        .execute(state.db())
        .await?;
    Ok(id)
}

/// Raison lisible d'un refus.
pub(crate) fn reason(e: &ApiError) -> String {
    match e {
        ApiError::EngineNotReady => "moteur non prêt (modèle en chargement ou arrêté)".into(),
        ApiError::NoActivePreset => "aucun preset actif".into(),
        ApiError::TurnInProgress => "un tour tourne déjà dans la session du cron".into(),
        ApiError::TooManySandboxes => "trop de sandboxes actives".into(),
        ApiError::SandboxUnavailable => "sandboxes indisponibles".into(),
        ApiError::VaultLocked => "coffre des clés verrouillé".into(),
        ApiError::NotFound => "topic introuvable".into(),
        ApiError::Internal(e) => format!("erreur interne : {e}"),
        other => other.to_string(),
    }
}

async fn run_prompt(state: &AppState, sid: &str, text: &str, timeout: Duration) -> (&'static str, String) {
    let started = now();
    if let Err(e) = sessions::start_turn(state, sid, text, Vec::new(), "cron").await {
        return ("failed", reason(&e));
    }
    let hub = bridge::hub(state, sid);
    let mut end = tokio::time::Instant::now() + timeout;
    // Tour, puis vérification et corrections éventuelles. L'attente dans la
    // file commune ne compte pas dans le délai.
    while hub.busy_or_settling() {
        if hub.queued() {
            end += Duration::from_millis(250);
        }
        if tokio::time::Instant::now() >= end {
            let _ = hub.interrupt().await;
            hub.cancel_verify().await;
            return (
                "failed",
                format!("délai dépassé ({} s) : tour interrompu", timeout.as_secs()),
            );
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    turns_settled(state, sid, started).await;
    let last: Option<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, status, error FROM chat_turns WHERE session_id = ? AND started_at >= ? \
         ORDER BY started_at DESC, rowid DESC LIMIT 1",
    )
    .bind(sid)
    .bind(started)
    .fetch_optional(state.db())
    .await
    .ok()
    .flatten();
    let answer: Option<String> = sqlx::query_scalar(
        "SELECT i.data FROM chat_items i JOIN chat_turns t ON t.session_id = i.session_id AND t.id = i.turn_id \
         WHERE i.session_id = ? AND i.kind = 'agentMessage' AND t.started_at >= ? ORDER BY i.seq DESC LIMIT 1",
    )
    .bind(sid)
    .bind(started)
    .fetch_optional(state.db())
    .await
    .ok()
    .flatten()
    .and_then(|d: String| serde_json::from_str::<Value>(&d).ok())
    .and_then(|v| v["text"].as_str().map(str::to_string));
    match last {
        Some((_, s, _)) if s == "completed" => ("ok", answer.unwrap_or_else(|| "(pas de réponse écrite)".into())),
        Some((_, s, err)) => (
            "failed",
            format!(
                "tour {s}{}{}",
                err.map(|e| format!(" : {e}")).unwrap_or_default(),
                answer.map(|a| format!("\n\n{a}")).unwrap_or_default()
            ),
        ),
        None => ("failed", "tour introuvable".into()),
    }
}

/// Le hub se libère juste avant que la fin du tour soit écrite en base :
/// attendre (10 s au plus) que plus aucun tour de la session ne soit en cours.
pub(crate) async fn turns_settled(state: &AppState, sid: &str, since: i64) {
    for _ in 0..40 {
        let open: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM chat_turns WHERE session_id = ? AND started_at >= ? AND status = 'inProgress'",
        )
        .bind(sid)
        .bind(since)
        .fetch_one(state.db())
        .await
        .unwrap_or(0);
        if open == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

async fn run_shell(state: &AppState, sid: &str, cmd: &str, timeout: Duration) -> (&'static str, String) {
    if let Err(e) = state.0.sandboxes.backend() {
        return ("failed", reason(&e));
    }
    if let Err(e) = sessions::make_room(state, sid).await {
        return ("failed", reason(&e));
    }
    let hub = bridge::hub(state, sid);
    let (client, cwd) = match hub.ready(state).await {
        Ok(x) => x,
        Err(e) => return ("failed", format!("sandbox : {e}")),
    };
    let ms = timeout.as_millis() as u64;
    let mut params = enginex_codex::types::command_exec(&["bash", "-lc", cmd], &cwd, None, None, Some(ms));
    params["outputBytesCap"] = json!(OUTPUT_MAX * 4);
    let r = client
        .request_with_timeout("command/exec", params, Some(timeout + Duration::from_secs(30)))
        .await;
    hub.touch_activity();
    match r {
        Ok(v) => {
            let code = v["exitCode"].as_i64().unwrap_or(-1);
            let out = format!(
                "{}{}",
                v["stdout"].as_str().unwrap_or(""),
                v["stderr"].as_str().unwrap_or("")
            );
            if code == 0 {
                ("ok", out)
            } else {
                ("failed", format!("{out}\n(code de sortie {code})"))
            }
        }
        Err(enginex_codex::Error::Timeout(_)) => ("failed", format!("délai dépassé ({} s)", timeout.as_secs())),
        Err(e) => ("failed", format!("command/exec : {e}")),
    }
}

/// Fin de la sortie (16 Kio), coupée sur un caractère.
pub(crate) fn tail(s: &str) -> String {
    if s.len() <= OUTPUT_MAX {
        return s.trim_end().to_string();
    }
    let mut start = s.len() - OUTPUT_MAX;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    format!("[… début coupé]\n{}", s[start..].trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paris(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        chrono_tz::Europe::Paris
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .single()
            .unwrap()
            .timestamp()
    }

    #[test]
    fn schedules() {
        let tz = parse_tz("Europe/Paris").unwrap();
        let s = parse_schedule("0 9 * * 1-5").unwrap();
        // Vendredi 20/11/2026 10:00 → lundi 23/11 9:00.
        assert_eq!(
            next_after(&s, tz, paris(2026, 11, 20, 10, 0)),
            Some(paris(2026, 11, 23, 9, 0))
        );
        // Juste à l'heure : la suivante, pas la même.
        assert_eq!(
            next_after(&s, tz, paris(2026, 11, 23, 9, 0)),
            Some(paris(2026, 11, 24, 9, 0))
        );
        // Passage à l'heure d'été (29/03/2026) : 9:00 de Paris reste 9:00.
        let s = parse_schedule("0 9 * * *").unwrap();
        let a = next_after(&s, tz, paris(2026, 3, 28, 12, 0)).unwrap();
        assert_eq!(a, paris(2026, 3, 29, 9, 0));
        assert_eq!(a - paris(2026, 3, 28, 9, 0), 23 * 3600);
        let e = parse_schedule("every:15").unwrap();
        assert_eq!(next_after(&e, tz, 1000), Some(1900));
        for bad in ["", "* * *", "every:0", "every:x", "61 * * * *", "0 9 * * * *"] {
            assert!(parse_schedule(bad).is_err(), "{bad}");
        }
        assert!(parse_tz("Mars/Olympus").is_err());
    }

    #[test]
    fn tails() {
        assert_eq!(tail("abc\n"), "abc");
        let long = "é".repeat(OUTPUT_MAX);
        let t = tail(&long);
        assert!(t.starts_with("[… début coupé]\n") && t.len() <= OUTPUT_MAX + 30);
    }
}
