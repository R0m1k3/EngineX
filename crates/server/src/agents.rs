//! Agents (étape 7c) : un rôle durable dans un topic, lancé par un horaire,
//! un webhook, une notification du fil, un autre agent (outil MCP
//! `agent_send`) ou à la main.
//!
//! Chaque départ ouvre une session neuve « Agent : <nom> » (les 20 dernières
//! restent visibles, les plus anciennes sont archivées) : le rôle et le
//! contexte du déclencheur forment le premier message, la continuité passe
//! par la mémoire du topic. Les tours passent par la file commune en
//! priorité de fond. Budget quotidien en jetons (`total.totalTokens` de
//! Codex) et en minutes de GPU (durée du départ hors file et hors attente
//! d'approbation) : atteint, plus de départ jusqu'à minuit (fuseau de
//! l'agent), une notification par jour.

use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
    time::{Duration, Instant},
};

use axum::{
    Json,
    extract::{Path as UrlPath, State},
};
use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    bridge, crons,
    error::{ApiError, ApiJson},
    feed::{self, LEVELS, Note},
    sessions,
    state::{AppState, now},
    workspace,
};

const RUNS_KEPT: i64 = 50;
/// Sessions de départ gardées visibles par agent.
const SESSIONS_KEPT: i64 = 20;
const NAME_MAX: usize = 80;
const ROLE_MAX: usize = 20_000;
const CONTEXT_MAX: usize = 8_000;
/// Profondeur maximale d'une chaîne d'agents (`agent_send`, fil).
pub const DEPTH_MAX: i64 = 3;
/// Écart minimal entre deux départs d'un agent par le fil.
const FEED_GAP: Duration = Duration::from_secs(60);
/// Approbation sans réponse (mode prudent) : tour interrompu.
const APPROVAL_WAIT: Duration = Duration::from_secs(3600);

/// État en mémoire : agents en cours, dernier départ par le fil.
#[derive(Default)]
pub struct Agents {
    running: Mutex<HashSet<i64>>,
    last_feed: Mutex<HashMap<i64, Instant>>,
}

impl Agents {
    fn take(&self, id: i64) -> bool {
        self.running.lock().expect("agents").insert(id)
    }
    fn release(&self, id: i64) {
        self.running.lock().expect("agents").remove(&id);
    }
    fn contains(&self, id: i64) -> bool {
        self.running.lock().expect("agents").contains(&id)
    }
}

// ---- Base ----

#[derive(Clone, Debug, sqlx::FromRow)]
pub struct AgentRow {
    pub id: i64,
    pub name: String,
    pub topic_id: i64,
    pub preset_id: Option<i64>,
    pub role: String,
    pub schedule: Option<String>,
    pub tz: String,
    pub feed_filter: Option<String>,
    pub approval: String,
    pub timeout_s: i64,
    pub budget_tokens: i64,
    pub budget_gpu_min: i64,
    pub notify: bool,
    pub enabled: bool,
    pub next_run: Option<i64>,
    pub budget_note_day: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

async fn row(state: &AppState, id: i64) -> Result<AgentRow, ApiError> {
    sqlx::query_as("SELECT * FROM agents WHERE id = ?")
        .bind(id)
        .fetch_optional(state.db())
        .await?
        .ok_or(ApiError::NotFound)
}

async fn by_name(state: &AppState, name: &str) -> Result<Option<AgentRow>, sqlx::Error> {
    sqlx::query_as("SELECT * FROM agents WHERE name = ? COLLATE NOCASE")
        .bind(name.trim())
        .fetch_optional(state.db())
        .await
}

/// Notifications du fil qui lancent l'agent.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FeedFilter {
    /// Sources retenues (cron, session, webhook, approval, agent…) ; vide = toutes.
    #[serde(default)]
    pub sources: Vec<String>,
    /// Niveau minimal : info < success < warning < error.
    #[serde(default = "info")]
    pub min_level: String,
    /// Texte cherché dans le titre ou le corps (sans casse) ; vide = tout.
    #[serde(default)]
    pub contains: String,
}

fn info() -> String {
    "info".into()
}

fn level_rank(l: &str) -> usize {
    LEVELS.iter().position(|x| *x == l).unwrap_or(0)
}

impl FeedFilter {
    fn matches(&self, source: &str, level: &str, title: &str, body: &str) -> bool {
        if !self.sources.is_empty() && !self.sources.iter().any(|s| s == source) {
            return false;
        }
        if level_rank(level) < level_rank(&self.min_level) {
            return false;
        }
        let needle = self.contains.trim().to_lowercase();
        needle.is_empty() || title.to_lowercase().contains(&needle) || body.to_lowercase().contains(&needle)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentBody {
    name: String,
    topic_id: i64,
    #[serde(default)]
    preset_id: Option<i64>,
    role: String,
    #[serde(default)]
    schedule: Option<String>,
    #[serde(default = "default_tz")]
    tz: String,
    #[serde(default)]
    feed_filter: Option<FeedFilter>,
    #[serde(default = "auto")]
    approval: String,
    #[serde(default = "default_timeout")]
    timeout_s: i64,
    #[serde(default)]
    budget_tokens: i64,
    #[serde(default)]
    budget_gpu_min: i64,
    #[serde(default)]
    notify: bool,
    #[serde(default = "yes")]
    enabled: bool,
}

fn default_tz() -> String {
    "Europe/Paris".into()
}
fn auto() -> String {
    "auto".into()
}
fn default_timeout() -> i64 {
    1800
}
fn yes() -> bool {
    true
}

fn bad(m: impl Into<String>) -> ApiError {
    ApiError::Invalid(m.into())
}

/// Réglages contrôlés ; rend l'horaire nettoyé et la prochaine exécution.
fn check(b: &AgentBody) -> Result<(Option<String>, Option<i64>), String> {
    let name = b.name.trim();
    if name.is_empty() || name.chars().count() > NAME_MAX || name.contains('\n') {
        return Err(format!("nom vide ou trop long ({NAME_MAX} caractères)"));
    }
    if b.role.trim().is_empty() || b.role.chars().count() > ROLE_MAX {
        return Err(format!("rôle vide ou trop long ({ROLE_MAX} caractères)"));
    }
    if !matches!(b.approval.as_str(), "auto" | "prudent") {
        return Err("approbation : auto ou prudent".into());
    }
    if !(60..=7200).contains(&b.timeout_s) {
        return Err("délai de 1 min à 2 h".into());
    }
    if b.budget_tokens < 0 || b.budget_gpu_min < 0 || b.budget_gpu_min > 1440 {
        return Err("budget : jetons ≥ 0, minutes de GPU de 0 à 1440".into());
    }
    if let Some(f) = &b.feed_filter
        && (!LEVELS.contains(&f.min_level.as_str()) || f.sources.len() > 20 || f.contains.chars().count() > 200)
    {
        return Err("filtre du fil invalide".into());
    }
    let tz = crons::parse_tz(&b.tz)?;
    let schedule = b
        .schedule
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let next = match &schedule {
        Some(s) => {
            let s = crons::parse_schedule(s)?;
            if b.enabled {
                crons::next_after(&s, tz, now())
            } else {
                None
            }
        }
        None => None,
    };
    Ok((schedule, next))
}

/// Début du jour (minuit local) dans le fuseau de l'agent, et sa date.
fn day_start(tz: &str, at: i64) -> (i64, String) {
    let tz = crons::parse_tz(tz).unwrap_or(chrono_tz::Europe::Paris);
    let local = Utc.timestamp_opt(at, 0).single().unwrap_or_default().with_timezone(&tz);
    let day = local.date_naive();
    let start = day
        .and_hms_opt(0, 0, 0)
        .and_then(|m| tz.from_local_datetime(&m).earliest())
        .map(|d| d.timestamp())
        .unwrap_or(at - 86_400);
    (start, day.format("%Y-%m-%d").to_string())
}

/// Jetons et secondes de GPU consommés aujourd'hui.
async fn used_today(state: &AppState, a: &AgentRow) -> (i64, i64) {
    let (start, _) = day_start(&a.tz, now());
    sqlx::query_as("SELECT COALESCE(SUM(tokens), 0), COALESCE(SUM(gpu_s), 0) FROM agent_runs WHERE agent_id = ? AND started_at >= ?")
        .bind(a.id)
        .bind(start)
        .fetch_one(state.db())
        .await
        .unwrap_or((0, 0))
}

/// Raison du refus si le budget du jour est atteint.
fn over_budget(a: &AgentRow, (tokens, gpu_s): (i64, i64)) -> Option<String> {
    if a.budget_tokens > 0 && tokens >= a.budget_tokens {
        return Some(format!(
            "budget du jour atteint : {tokens} jetons sur {}",
            a.budget_tokens
        ));
    }
    if a.budget_gpu_min > 0 && gpu_s >= a.budget_gpu_min * 60 {
        return Some(format!(
            "budget du jour atteint : {} min de GPU sur {}",
            gpu_s / 60,
            a.budget_gpu_min
        ));
    }
    None
}

// ---- API ----

/// Statut, déclencheur, début, fin, session.
type LastRun = (String, String, i64, Option<i64>, Option<String>);

async fn agent_json(state: &AppState, a: &AgentRow) -> Result<Value, ApiError> {
    let last: Option<LastRun> = sqlx::query_as(
        "SELECT status, trigger, started_at, ended_at, session_id FROM agent_runs WHERE agent_id = ? AND status <> 'skipped' ORDER BY id DESC LIMIT 1",
    )
    .bind(a.id)
    .fetch_optional(state.db())
    .await?;
    let topic = workspace::topic(state, a.topic_id).await.ok();
    let (tokens, gpu_s) = used_today(state, a).await;
    let hooks: Vec<(i64, String)> = sqlx::query_as("SELECT id, name FROM webhooks WHERE agent_id = ? ORDER BY name")
        .bind(a.id)
        .fetch_all(state.db())
        .await?;
    Ok(json!({
        "id": a.id, "name": a.name, "topic_id": a.topic_id,
        "topic": topic.map(|t| format!("{} / {}", t.project_slug, t.slug)),
        "preset_id": a.preset_id, "role": a.role, "schedule": a.schedule, "tz": a.tz,
        "feed_filter": a.feed_filter.as_deref().and_then(|f| serde_json::from_str::<Value>(f).ok()),
        "approval": a.approval, "timeout_s": a.timeout_s,
        "budget_tokens": a.budget_tokens, "budget_gpu_min": a.budget_gpu_min,
        "notify": a.notify, "enabled": a.enabled, "next_run": a.next_run,
        "running": state.0.agents.contains(a.id),
        "today": {"tokens": tokens, "gpu_s": gpu_s},
        "webhooks": hooks.into_iter().map(|(id, name)| json!({"id": id, "name": name})).collect::<Vec<_>>(),
        "last": last.map(|(s, t, a, e, sid)| json!({"status": s, "trigger": t, "started_at": a, "ended_at": e, "session_id": sid})),
    }))
}

pub async fn list(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let rows: Vec<AgentRow> = sqlx::query_as("SELECT * FROM agents ORDER BY name COLLATE NOCASE")
        .fetch_all(state.db())
        .await?;
    let mut out = Vec::new();
    for a in &rows {
        out.push(agent_json(&state, a).await?);
    }
    Ok(Json(Value::Array(out)))
}

async fn checked(state: &AppState, b: &AgentBody, id: Option<i64>) -> Result<(Option<String>, Option<i64>), ApiError> {
    let r = check(b).map_err(bad)?;
    workspace::topic(state, b.topic_id).await?;
    sessions::check_preset(state, b.preset_id).await?;
    if let Some(o) = by_name(state, &b.name).await?
        && Some(o.id) != id
    {
        return Err(bad("un agent porte déjà ce nom"));
    }
    Ok(r)
}

fn filter_json(b: &AgentBody) -> Option<String> {
    b.feed_filter
        .as_ref()
        .map(|f| serde_json::to_string(f).unwrap_or_default())
}

pub async fn create(State(state): State<AppState>, ApiJson(b): ApiJson<AgentBody>) -> Result<Json<Value>, ApiError> {
    let (schedule, next) = checked(&state, &b, None).await?;
    let t = now();
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO agents (name, topic_id, preset_id, role, schedule, tz, feed_filter, approval, timeout_s, \
         budget_tokens, budget_gpu_min, notify, enabled, next_run, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(b.name.trim())
    .bind(b.topic_id)
    .bind(b.preset_id)
    .bind(b.role.trim())
    .bind(&schedule)
    .bind(b.tz.trim())
    .bind(filter_json(&b))
    .bind(&b.approval)
    .bind(b.timeout_s)
    .bind(b.budget_tokens)
    .bind(b.budget_gpu_min)
    .bind(b.notify)
    .bind(b.enabled)
    .bind(next)
    .bind(t)
    .bind(t)
    .fetch_one(state.db())
    .await?;
    Ok(Json(json!({"id": id, "next_run": next})))
}

pub async fn update(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<i64>,
    ApiJson(b): ApiJson<AgentBody>,
) -> Result<Json<Value>, ApiError> {
    row(&state, id).await?;
    let (schedule, next) = checked(&state, &b, Some(id)).await?;
    sqlx::query(
        "UPDATE agents SET name = ?, topic_id = ?, preset_id = ?, role = ?, schedule = ?, tz = ?, feed_filter = ?, \
         approval = ?, timeout_s = ?, budget_tokens = ?, budget_gpu_min = ?, notify = ?, enabled = ?, next_run = ?, \
         updated_at = ? WHERE id = ?",
    )
    .bind(b.name.trim())
    .bind(b.topic_id)
    .bind(b.preset_id)
    .bind(b.role.trim())
    .bind(&schedule)
    .bind(b.tz.trim())
    .bind(filter_json(&b))
    .bind(&b.approval)
    .bind(b.timeout_s)
    .bind(b.budget_tokens)
    .bind(b.budget_gpu_min)
    .bind(b.notify)
    .bind(b.enabled)
    .bind(next)
    .bind(now())
    .bind(id)
    .execute(state.db())
    .await?;
    Ok(Json(json!({"id": id, "next_run": next})))
}

pub async fn delete(State(state): State<AppState>, UrlPath(id): UrlPath<i64>) -> Result<Json<Value>, ApiError> {
    row(&state, id).await?;
    if state.0.agents.contains(id) {
        return Err(ApiError::Busy);
    }
    sqlx::query("DELETE FROM agents WHERE id = ?")
        .bind(id)
        .execute(state.db())
        .await?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunBody {
    #[serde(default)]
    context: String,
}

pub async fn run_now(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<i64>,
    ApiJson(b): ApiJson<RunBody>,
) -> Result<Json<Value>, ApiError> {
    let a = row(&state, id).await?;
    match start(&state, a, "manual", b.context, 0).await {
        Started::Run(run) => Ok(Json(json!({"started": true, "run_id": run}))),
        Started::Busy => Err(ApiError::Busy),
        Started::Refused(m) => Err(bad(m)),
    }
}

/// Coupe le départ en cours (tour interrompu, vérification annulée).
pub async fn stop(State(state): State<AppState>, UrlPath(id): UrlPath<i64>) -> Result<Json<Value>, ApiError> {
    row(&state, id).await?;
    let sid: Option<String> = sqlx::query_scalar(
        "SELECT session_id FROM agent_runs WHERE agent_id = ? AND status = 'running' ORDER BY id DESC LIMIT 1",
    )
    .bind(id)
    .fetch_optional(state.db())
    .await?
    .flatten();
    let Some(sid) = sid else {
        return Ok(Json(json!({"stopped": false})));
    };
    let hub = bridge::hub(&state, &sid);
    let _ = hub.interrupt().await;
    hub.cancel_verify().await;
    Ok(Json(json!({"stopped": true})))
}

#[derive(Serialize, sqlx::FromRow)]
pub struct RunRow {
    id: i64,
    trigger: String,
    context: String,
    status: String,
    started_at: i64,
    ended_at: Option<i64>,
    output: String,
    session_id: Option<String>,
    tokens: i64,
    gpu_s: i64,
    depth: i64,
}

pub async fn runs(State(state): State<AppState>, UrlPath(id): UrlPath<i64>) -> Result<Json<Vec<RunRow>>, ApiError> {
    row(&state, id).await?;
    let rows: Vec<RunRow> = sqlx::query_as(
        "SELECT id, trigger, context, status, started_at, ended_at, output, session_id, tokens, gpu_s, depth \
         FROM agent_runs WHERE agent_id = ? ORDER BY id DESC LIMIT ?",
    )
    .bind(id)
    .bind(RUNS_KEPT)
    .fetch_all(state.db())
    .await?;
    Ok(Json(rows))
}

// ---- Déclencheurs ----

pub fn spawn(state: &AppState) {
    let state = state.clone();
    tokio::spawn(async move {
        // Départs coupés par un arrêt d'EngineX.
        let _ = sqlx::query(
            "UPDATE agent_runs SET status = 'failed', ended_at = ?, output = 'EngineX a redémarré pendant le départ' \
             WHERE status = 'running'",
        )
        .bind(now())
        .execute(state.db())
        .await;
        let tick = state.config().timings.cron_tick;
        loop {
            if let Err(e) = tick_once(&state).await {
                tracing::warn!("agents : {e}");
            }
            tokio::time::sleep(tick).await;
        }
    });
}

/// Lance les agents dont l'horaire est échu (un seul rattrapage après un arrêt).
pub async fn tick_once(state: &AppState) -> Result<(), sqlx::Error> {
    let t = now();
    let due: Vec<AgentRow> = sqlx::query_as(
        "SELECT * FROM agents WHERE enabled = 1 AND schedule IS NOT NULL AND next_run IS NOT NULL AND next_run <= ?",
    )
    .bind(t)
    .fetch_all(state.db())
    .await?;
    for a in due {
        let next = match (
            crons::parse_schedule(a.schedule.as_deref().unwrap_or("")),
            crons::parse_tz(&a.tz),
        ) {
            (Ok(s), Ok(tz)) => crons::next_after(&s, tz, t),
            _ => None,
        };
        sqlx::query("UPDATE agents SET next_run = ? WHERE id = ?")
            .bind(next)
            .bind(a.id)
            .execute(state.db())
            .await?;
        let s = state.clone();
        tokio::spawn(async move {
            start(&s, a, "schedule", String::new(), 0).await;
        });
    }
    Ok(())
}

/// Une notification vient d'être postée : lance les agents dont le filtre la
/// retient. Jamais l'agent qui l'a produite ; une minute au moins entre deux
/// départs par le fil ; profondeur de chaîne bornée.
pub async fn on_note(state: &AppState, note: &Note, title: &str, body: &str) {
    let depth = note.agent.map(|(_, d)| d + 1).unwrap_or(0);
    if depth > DEPTH_MAX {
        return;
    }
    let Ok(list): Result<Vec<AgentRow>, _> =
        sqlx::query_as("SELECT * FROM agents WHERE enabled = 1 AND feed_filter IS NOT NULL")
            .fetch_all(state.db())
            .await
    else {
        return;
    };
    for a in list {
        if note.agent.map(|(id, _)| id) == Some(a.id) {
            continue;
        }
        let Some(f) = a
            .feed_filter
            .as_deref()
            .and_then(|f| serde_json::from_str::<FeedFilter>(f).ok())
        else {
            continue;
        };
        if !f.matches(note.source, note.level, title, body) {
            continue;
        }
        {
            let mut last = state.0.agents.last_feed.lock().expect("agents");
            if last.get(&a.id).is_some_and(|t| t.elapsed() < FEED_GAP) {
                continue;
            }
            last.insert(a.id, Instant::now());
        }
        let context = format!(
            "Notification du fil ({}, {}) : {title}{}",
            note.source,
            note.level,
            if body.is_empty() {
                String::new()
            } else {
                format!("\n\n{body}")
            }
        );
        let s = state.clone();
        tokio::spawn(async move {
            start(&s, a, "feed", context, depth).await;
        });
    }
}

/// Appel entrant d'un webhook lié à l'agent `id`.
pub async fn on_webhook(state: &AppState, id: i64, hook: &str, title: &str, body: &str) {
    let Ok(a) = row(state, id).await else { return };
    if !a.enabled {
        return;
    }
    let context = format!(
        "Webhook « {hook} » : {title}{}",
        if body.is_empty() {
            String::new()
        } else {
            format!("\n\n{body}")
        }
    );
    start(state, a, "webhook", context, 0).await;
}

/// Départ demandé par une session (outil MCP `agent_send`). La profondeur
/// suit la chaîne si la session appelante est elle-même un départ d'agent.
pub async fn send_from_session(state: &AppState, sid: &str, name: &str, message: &str) -> Result<String, String> {
    let a = by_name(state, name)
        .await
        .map_err(|e| format!("base : {e}"))?
        .ok_or_else(|| format!("agent inconnu : {name} (voir agent_list)"))?;
    let me: Option<(i64, i64)> =
        sqlx::query_as("SELECT agent_id, depth FROM agent_runs WHERE session_id = ? ORDER BY id DESC LIMIT 1")
            .bind(sid)
            .fetch_optional(state.db())
            .await
            .map_err(|e| format!("base : {e}"))?;
    if me.map(|(id, _)| id) == Some(a.id) {
        return Err("un agent ne se confie pas de travail à lui-même".into());
    }
    let (trigger, depth) = match me {
        Some((_, d)) => ("agent", d + 1),
        None => ("session", 0),
    };
    if depth > DEPTH_MAX {
        return Err(format!("chaîne d'agents trop longue ({DEPTH_MAX} niveaux au plus)"));
    }
    if !a.enabled {
        return Err(format!("l'agent « {} » est coupé", a.name));
    }
    let from = match me {
        Some((id, _)) => row(state, id)
            .await
            .map(|r| format!("l'agent « {} »", r.name))
            .unwrap_or_default(),
        None => "une session".into(),
    };
    let name = a.name.clone();
    match start(
        state,
        a,
        trigger,
        format!("Travail confié par {from} :\n\n{message}"),
        depth,
    )
    .await
    {
        Started::Run(id) => Ok(format!(
            "Départ {id} de l'agent « {name} » lancé ; le résultat arrivera dans le fil."
        )),
        Started::Busy => Err(format!("l'agent « {name} » tourne déjà ; réessaie plus tard")),
        Started::Refused(m) => Err(m),
    }
}

/// Liste lisible pour l'outil MCP `agent_list`.
pub async fn list_text(state: &AppState) -> Result<String, String> {
    let rows: Vec<AgentRow> = sqlx::query_as("SELECT * FROM agents ORDER BY name COLLATE NOCASE")
        .fetch_all(state.db())
        .await
        .map_err(|e| format!("base : {e}"))?;
    if rows.is_empty() {
        return Ok("Aucun agent.".into());
    }
    let mut out = String::new();
    for a in rows {
        let first = a.role.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        let first: String = first.chars().take(160).collect();
        out.push_str(&format!(
            "- {}{}{} : {first}\n",
            a.name,
            if a.enabled { "" } else { " (coupé)" },
            if state.0.agents.contains(a.id) {
                " (en cours)"
            } else {
                ""
            },
        ));
    }
    Ok(out)
}

// ---- Exécution ----

pub enum Started {
    Run(i64),
    Busy,
    Refused(String),
}

async fn insert_run(
    state: &AppState,
    a: i64,
    trigger: &str,
    context: &str,
    status: &str,
    output: &str,
    depth: i64,
) -> Option<i64> {
    let t = now();
    let id = sqlx::query_scalar(
        "INSERT INTO agent_runs (agent_id, trigger, context, status, started_at, ended_at, output, depth) \
         VALUES (?, ?, ?, ?, ?, CASE WHEN ? = 'running' THEN NULL ELSE ? END, ?, ?) RETURNING id",
    )
    .bind(a)
    .bind(trigger)
    .bind(context)
    .bind(status)
    .bind(t)
    .bind(status)
    .bind(t)
    .bind(output)
    .bind(depth)
    .fetch_one(state.db())
    .await
    .map_err(|e| tracing::warn!("agents : {e}"))
    .ok();
    let _ = sqlx::query(
        "DELETE FROM agent_runs WHERE agent_id = ? AND id NOT IN \
         (SELECT id FROM agent_runs WHERE agent_id = ? ORDER BY id DESC LIMIT ?)",
    )
    .bind(a)
    .bind(a)
    .bind(RUNS_KEPT)
    .execute(state.db())
    .await;
    id
}

fn cut(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut o: String = s.chars().take(max).collect();
    o.push('…');
    o
}

/// Contrôles (budget, un départ à la fois), puis départ en fond.
pub async fn start(state: &AppState, a: AgentRow, trigger: &'static str, context: String, depth: i64) -> Started {
    let context = cut(&state.masker().mask(context.trim()), CONTEXT_MAX);
    if let Some(why) = over_budget(&a, used_today(state, &a).await) {
        insert_run(state, a.id, trigger, &context, "skipped", &why, depth).await;
        budget_note(state, &a, &why).await;
        return Started::Refused(why);
    }
    if !state.0.agents.take(a.id) {
        insert_run(
            state,
            a.id,
            trigger,
            &context,
            "skipped",
            "le départ précédent tourne encore",
            depth,
        )
        .await;
        return Started::Busy;
    }
    let Some(run) = insert_run(state, a.id, trigger, &context, "running", "", depth).await else {
        state.0.agents.release(a.id);
        return Started::Refused("erreur interne".into());
    };
    tokio::spawn(run_taken(state.clone(), a, run, trigger, context, depth));
    Started::Run(run)
}

/// Une notification par jour quand le budget est atteint.
async fn budget_note(state: &AppState, a: &AgentRow, why: &str) {
    let (_, day) = day_start(&a.tz, now());
    let changed = sqlx::query(
        "UPDATE agents SET budget_note_day = ? WHERE id = ? AND (budget_note_day IS NULL OR budget_note_day <> ?)",
    )
    .bind(&day)
    .bind(a.id)
    .bind(&day)
    .execute(state.db())
    .await
    .map(|r| r.rows_affected() > 0)
    .unwrap_or(false);
    if changed {
        feed::emit(
            state,
            Note::new("agent", "warning", format!("Agent « {} » : budget atteint", a.name))
                .body(format!("{why}. Plus de départ jusqu'à minuit."))
                .link(format!("#agents/{}", a.id))
                .important(true)
                .from_agent(a.id, DEPTH_MAX),
        );
    }
}

fn trigger_text(trigger: &str) -> &'static str {
    match trigger {
        "schedule" => "horaire",
        "manual" => "lancé à la main par Michael",
        "webhook" => "webhook",
        "feed" => "notification du fil",
        "agent" => "un autre agent",
        "session" => "une session",
        _ => "inconnu",
    }
}

fn first_message(a: &AgentRow, trigger: &str, context: &str) -> String {
    let when = crons::parse_tz(&a.tz)
        .ok()
        .and_then(|tz| Utc.timestamp_opt(now(), 0).single().map(|d| d.with_timezone(&tz)))
        .map(|d| d.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default();
    let mut m = format!(
        "Tu es l'agent « {} ». Ton rôle :\n\n{}\n\n---\nDépart : {} ({when}).",
        a.name,
        a.role.trim(),
        trigger_text(trigger)
    );
    if !context.is_empty() {
        m.push_str(&format!("\n\n{context}"));
    }
    m.push_str(
        "\n\nTravaille seul : personne ne lit la session en direct. Garde dans la mémoire du topic ce qui doit \
         servir aux prochains départs. Termine par un compte rendu court (il part dans le fil de Michael).",
    );
    m
}

async fn new_session(state: &AppState, a: &AgentRow) -> Result<String, ApiError> {
    workspace::topic(state, a.topic_id).await?;
    let id = uuid::Uuid::new_v4().to_string();
    let t = now();
    sqlx::query(
        "INSERT INTO chat_sessions (id, topic_id, title, approval, created_at, updated_at, preset_id) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(a.topic_id)
    .bind(format!("Agent : {}", a.name))
    .bind(&a.approval)
    .bind(t)
    .bind(t)
    .bind(a.preset_id)
    .execute(state.db())
    .await?;
    Ok(id)
}

struct Outcome {
    status: &'static str,
    output: String,
    tokens: i64,
    gpu_s: i64,
}

async fn run_taken(state: AppState, a: AgentRow, run: i64, trigger: &'static str, context: String, depth: i64) {
    let (out, sid) = match new_session(&state, &a).await {
        Err(e) => (
            Outcome {
                status: "failed",
                output: format!("session de l'agent : {}", crons::reason(&e)),
                tokens: 0,
                gpu_s: 0,
            },
            None,
        ),
        Ok(sid) => {
            let _ = sqlx::query("UPDATE agent_runs SET session_id = ? WHERE id = ?")
                .bind(&sid)
                .bind(run)
                .execute(state.db())
                .await;
            let o = drive(&state, &a, &sid, &first_message(&a, trigger, &context)).await;
            (o, Some(sid))
        }
    };
    let output = crons::tail(&state.masker().mask(&out.output));
    let _ =
        sqlx::query("UPDATE agent_runs SET status = ?, ended_at = ?, output = ?, tokens = ?, gpu_s = ? WHERE id = ?")
            .bind(out.status)
            .bind(now())
            .bind(&output)
            .bind(out.tokens)
            .bind(out.gpu_s)
            .bind(run)
            .execute(state.db())
            .await;
    state.0.agents.release(a.id);
    archive_old(&state, a.id).await;
    tracing::info!("agent « {} » ({trigger}) : {}", a.name, out.status);
    let ok = out.status == "ok";
    let short: String = {
        let rev: Vec<char> = output.chars().rev().take(600).collect();
        rev.into_iter().rev().collect()
    };
    let mut note = Note::new(
        "agent",
        if ok { "success" } else { "error" },
        format!("Agent « {} » : {}", a.name, if ok { "fini" } else { "en échec" }),
    )
    .body(short)
    .important(!ok || a.notify)
    .from_agent(a.id, depth);
    note = match &sid {
        Some(s) => note.link(format!("#sessions/{s}")),
        None => note.link(format!("#agents/{}", a.id)),
    };
    feed::emit(&state, note);
}

/// Premier tour, vérification et corrections ; rend le résultat du départ.
async fn drive(state: &AppState, a: &AgentRow, sid: &str, text: &str) -> Outcome {
    let failed = |m: String| Outcome {
        status: "failed",
        output: m,
        tokens: 0,
        gpu_s: 0,
    };
    let started = now();
    if let Err(e) = sessions::start_turn(state, sid, text, Vec::new(), "agent").await {
        return failed(crons::reason(&e));
    }
    let hub = bridge::hub(state, sid);
    let timeout = Duration::from_secs(a.timeout_s.max(60) as u64);
    let step = Duration::from_millis(250);
    let begun = Instant::now();
    let (mut paused, mut asked) = (Duration::ZERO, Duration::ZERO);
    let mut note = String::new();
    while hub.busy() {
        if hub.queued() {
            paused += step;
        } else if hub.waiting_on_user() {
            paused += step;
            asked += step;
            if a.approval == "prudent" && asked >= APPROVAL_WAIT {
                note = "sans réponse à l'approbation en 1 h : tour interrompu".into();
                let _ = hub.interrupt().await;
                hub.cancel_verify().await;
                break;
            }
        } else {
            asked = Duration::ZERO;
        }
        if begun.elapsed().saturating_sub(paused) >= timeout {
            note = format!("délai dépassé ({} s) : tour interrompu", timeout.as_secs());
            let _ = hub.interrupt().await;
            hub.cancel_verify().await;
            break;
        }
        tokio::time::sleep(step).await;
    }
    // Interruption demandée : attendre que le tour se ferme (borné).
    for _ in 0..40 {
        if !hub.busy() {
            break;
        }
        tokio::time::sleep(step).await;
    }
    let gpu_s = begun.elapsed().saturating_sub(paused).as_secs() as i64;
    let turns: Vec<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT status, error, usage FROM chat_turns WHERE session_id = ? AND started_at >= ? \
         ORDER BY started_at DESC, rowid DESC",
    )
    .bind(sid)
    .bind(started)
    .fetch_all(state.db())
    .await
    .unwrap_or_default();
    // Compteur cumulé du fil Codex : le plus grand vu.
    let tokens = turns
        .iter()
        .filter_map(|(_, _, u)| u.as_deref().and_then(|u| serde_json::from_str::<Value>(u).ok()))
        .filter_map(|u| u["total"]["totalTokens"].as_i64())
        .max()
        .unwrap_or(0);
    let answer: Option<String> = sqlx::query_scalar(
        "SELECT i.data FROM chat_items i JOIN chat_turns t ON t.session_id = i.session_id AND t.id = i.turn_id \
         WHERE i.session_id = ? AND i.kind = 'agentMessage' ORDER BY i.seq DESC LIMIT 1",
    )
    .bind(sid)
    .fetch_optional(state.db())
    .await
    .ok()
    .flatten()
    .and_then(|d: String| serde_json::from_str::<Value>(&d).ok())
    .and_then(|v| v["text"].as_str().map(str::to_string));
    let (status, output) = match turns.first() {
        Some((s, _, _)) if s == "completed" && note.is_empty() => {
            ("ok", answer.unwrap_or_else(|| "(pas de compte rendu écrit)".into()))
        }
        Some((s, err, _)) => (
            "failed",
            format!(
                "{}tour {s}{}{}",
                if note.is_empty() {
                    String::new()
                } else {
                    format!("{note}\n")
                },
                err.as_ref().map(|e| format!(" : {e}")).unwrap_or_default(),
                answer.map(|a| format!("\n\n{a}")).unwrap_or_default()
            ),
        ),
        None => (
            "failed",
            if note.is_empty() {
                "tour introuvable".into()
            } else {
                note
            },
        ),
    };
    Outcome {
        status,
        output,
        tokens,
        gpu_s,
    }
}

/// Archive les sessions des départs au-delà des 20 derniers.
async fn archive_old(state: &AppState, agent: i64) {
    let old: Vec<String> = sqlx::query_scalar(
        "SELECT s.id FROM chat_sessions s WHERE s.archived = 0 AND s.id IN \
         (SELECT session_id FROM agent_runs WHERE agent_id = ? AND session_id IS NOT NULL AND status <> 'running' \
          ORDER BY id DESC LIMIT -1 OFFSET ?)",
    )
    .bind(agent)
    .bind(SESSIONS_KEPT)
    .fetch_all(state.db())
    .await
    .unwrap_or_default();
    for sid in old {
        let _ = sqlx::query("UPDATE chat_sessions SET archived = 1 WHERE id = ?")
            .bind(&sid)
            .execute(state.db())
            .await;
        bridge::hub(state, &sid).stop_conn(state, "session archivée").await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters() {
        let f = FeedFilter {
            sources: vec!["webhook".into()],
            min_level: "warning".into(),
            contains: "Vente".into(),
        };
        assert!(f.matches("webhook", "error", "boutique : vente", ""));
        assert!(f.matches("webhook", "warning", "x", "nouvelle VENTE"));
        assert!(!f.matches("cron", "error", "vente", ""));
        assert!(!f.matches("webhook", "info", "vente", ""));
        assert!(!f.matches("webhook", "error", "rien", ""));
        assert!(
            FeedFilter {
                min_level: "info".into(),
                ..Default::default()
            }
            .matches("disk", "info", "", "")
        );
    }

    #[test]
    fn days() {
        // 19/11/2026 23:30 à Paris (UTC+1) = 22:30 UTC.
        let t = chrono_tz::Europe::Paris
            .with_ymd_and_hms(2026, 11, 19, 23, 30, 0)
            .single()
            .unwrap()
            .timestamp();
        let (start, day) = day_start("Europe/Paris", t);
        assert_eq!(day, "2026-11-19");
        assert_eq!(t - start, 23 * 3600 + 1800);
    }

    #[test]
    fn budgets() {
        let mut a = AgentRow {
            id: 1,
            name: "a".into(),
            topic_id: 1,
            preset_id: None,
            role: "r".into(),
            schedule: None,
            tz: "Europe/Paris".into(),
            feed_filter: None,
            approval: "auto".into(),
            timeout_s: 600,
            budget_tokens: 0,
            budget_gpu_min: 0,
            notify: false,
            enabled: true,
            next_run: None,
            budget_note_day: None,
            created_at: 0,
            updated_at: 0,
        };
        assert!(over_budget(&a, (1_000_000, 100_000)).is_none());
        a.budget_tokens = 1000;
        assert!(over_budget(&a, (999, 0)).is_none());
        assert!(over_budget(&a, (1000, 0)).is_some());
        a.budget_tokens = 0;
        a.budget_gpu_min = 2;
        assert!(over_budget(&a, (0, 119)).is_none());
        assert!(over_budget(&a, (0, 120)).unwrap().contains("GPU"));
    }
}
