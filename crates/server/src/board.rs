//! Tableau des tâches (kanban) : `<data>/workspace/tasks.json`, même forme
//! que celui de Harness Maxxing. Lu et écrit en entier sous `board_lock`,
//! écriture atomique ; les champs inconnus sont gardés tels quels.
//! Partagé par l'API de l'UI et les outils MCP `tasks_*` de Codex.

use std::path::PathBuf;

use axum::{
    Json,
    extract::{Path as UrlPath, State},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::{
    error::{ApiError, ApiJson},
    state::{AppState, now},
};

const MAX_TASKS: usize = 5000;
const MAX_PROJECTS: usize = 200;
const TITLE_MAX: usize = 300;
const NOTES_MAX: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Todo,
    Doing,
    Done,
}

impl Status {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "todo" => Some(Self::Todo),
            "doing" => Some(Self::Doing),
            "done" => Some(Self::Done),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Todo => "à faire",
            Self::Doing => "en cours",
            Self::Done => "fait",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub notes: String,
    pub status: Status,
    #[serde(default)]
    pub project_id: Option<String>,
    /// Échéance `AAAA-MM-JJ`.
    #[serde(default)]
    pub due: Option<String>,
    /// Rang dans la colonne (croissant).
    #[serde(default)]
    pub order: f64,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_at: Option<i64>,
    /// Qui l'a créée : `michael` ou `session:<id>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskProject {
    pub id: String,
    pub name: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Board {
    pub version: u32,
    #[serde(default)]
    pub projects: Vec<TaskProject>,
    #[serde(default)]
    pub tasks: Vec<Task>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl Default for Board {
    fn default() -> Self {
        Self {
            version: 1,
            projects: Vec::new(),
            tasks: Vec::new(),
            extra: Map::new(),
        }
    }
}

/// Refus d'une modification (message pour l'UI ou le modèle).
#[derive(Debug)]
pub enum Refusal {
    NotFound(String),
    Invalid(String),
    Internal(anyhow::Error),
}

impl From<Refusal> for ApiError {
    fn from(r: Refusal) -> Self {
        match r {
            Refusal::NotFound(_) => ApiError::NotFound,
            Refusal::Invalid(_) => ApiError::BadRequest,
            Refusal::Internal(e) => ApiError::Internal(e),
        }
    }
}

impl Refusal {
    pub fn message(self) -> String {
        match self {
            Self::NotFound(m) | Self::Invalid(m) => m,
            Self::Internal(e) => {
                tracing::error!("tâches : {e:#}");
                "erreur interne d'EngineX".into()
            }
        }
    }
}

fn invalid(m: impl Into<String>) -> Refusal {
    Refusal::Invalid(m.into())
}

pub fn path(state: &AppState) -> PathBuf {
    state.config().data_dir.join("workspace/tasks.json")
}

fn load(state: &AppState) -> Result<Board, Refusal> {
    match std::fs::read_to_string(path(state)) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| Refusal::Internal(anyhow::anyhow!("tasks.json illisible : {e}"))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Board::default()),
        Err(e) => Err(Refusal::Internal(e.into())),
    }
}

/// Lecture seule.
pub async fn read(state: &AppState) -> Result<Board, Refusal> {
    let _g = state.0.board_lock.lock().await;
    load(state)
}

/// Lecture, modification par `f`, écriture (si `f` réussit).
pub async fn update<T>(state: &AppState, f: impl FnOnce(&mut Board) -> Result<T, Refusal>) -> Result<T, Refusal> {
    let _g = state.0.board_lock.lock().await;
    let mut b = load(state)?;
    let out = f(&mut b)?;
    let data = serde_json::to_string_pretty(&b).map_err(|e| Refusal::Internal(e.into()))?;
    crate::memory::write_atomic(&path(state), &data).map_err(|e| Refusal::Internal(e.into()))?;
    Ok(out)
}

fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..10].to_string()
}

pub fn check_title(t: &str) -> Result<String, Refusal> {
    let t = t.trim();
    if t.is_empty() {
        return Err(invalid("le titre est vide"));
    }
    if t.contains('\n') {
        return Err(invalid("le titre tient sur une ligne"));
    }
    if t.chars().count() > TITLE_MAX {
        return Err(invalid(format!("titre trop long ({TITLE_MAX} caractères au plus)")));
    }
    Ok(t.to_string())
}

fn check_notes(n: &str) -> Result<String, Refusal> {
    if n.chars().count() > NOTES_MAX {
        return Err(invalid(format!("notes trop longues ({NOTES_MAX} caractères au plus)")));
    }
    Ok(n.trim_end().to_string())
}

pub fn check_due(d: &str) -> Result<String, Refusal> {
    chrono::NaiveDate::parse_from_str(d.trim(), "%Y-%m-%d")
        .map(|d| d.format("%Y-%m-%d").to_string())
        .map_err(|_| invalid(format!("échéance {d:?} : format AAAA-MM-JJ attendu")))
}

impl Board {
    pub fn task_mut(&mut self, id: &str) -> Result<&mut Task, Refusal> {
        self.tasks
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or_else(|| Refusal::NotFound(format!("tâche {id} introuvable")))
    }

    pub fn project_name(&self, id: Option<&str>) -> Option<&str> {
        let id = id?;
        self.projects.iter().find(|p| p.id == id).map(|p| p.name.as_str())
    }

    /// Projet par son nom (casse ignorée), créé s'il n'existe pas.
    pub fn project_by_name(&mut self, name: &str) -> Result<String, Refusal> {
        let name = name.trim();
        if let Some(p) = self.projects.iter().find(|p| p.name.eq_ignore_ascii_case(name)) {
            return Ok(p.id.clone());
        }
        self.add_project(name)
    }

    pub fn add_project(&mut self, name: &str) -> Result<String, Refusal> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 80 || name.contains('\n') {
            return Err(invalid("nom de projet vide ou trop long (80 caractères)"));
        }
        if self.projects.iter().any(|p| p.name.eq_ignore_ascii_case(name)) {
            return Err(invalid(format!("le projet « {name} » existe déjà")));
        }
        if self.projects.len() >= MAX_PROJECTS {
            return Err(invalid("trop de projets de tâches"));
        }
        let id = new_id();
        self.projects.push(TaskProject {
            id: id.clone(),
            name: name.to_string(),
            extra: Map::new(),
        });
        Ok(id)
    }

    fn last_order(&self, status: Status) -> f64 {
        self.tasks
            .iter()
            .filter(|t| t.status == status)
            .map(|t| t.order)
            .fold(0.0, f64::max)
    }

    pub fn add_task(&mut self, new: NewTask, by: &str) -> Result<Task, Refusal> {
        if self.tasks.len() >= MAX_TASKS {
            return Err(invalid("trop de tâches : archive ou supprime celles qui sont faites"));
        }
        let status = new.status.unwrap_or(Status::Todo);
        if let Some(p) = &new.project_id
            && !self.projects.iter().any(|x| &x.id == p)
        {
            return Err(Refusal::NotFound(format!("projet de tâches {p} introuvable")));
        }
        let t = now();
        let task = Task {
            id: new_id(),
            title: check_title(&new.title)?,
            notes: check_notes(new.notes.as_deref().unwrap_or(""))?,
            status,
            project_id: new.project_id,
            due: new.due.as_deref().map(check_due).transpose()?,
            order: self.last_order(status) + 1.0,
            created_at: t,
            updated_at: t,
            done_at: (status == Status::Done).then_some(t),
            created_by: Some(by.to_string()),
            extra: Map::new(),
        };
        self.tasks.push(task.clone());
        Ok(task)
    }

    /// Applique un patch JSON : `title`, `notes`, `status`, `project_id`
    /// (null = sans), `due` (null = sans), `order`.
    pub fn patch_task(&mut self, id: &str, p: &Map<String, Value>) -> Result<Task, Refusal> {
        for k in p.keys() {
            if !matches!(
                k.as_str(),
                "title" | "notes" | "status" | "project_id" | "due" | "order"
            ) {
                return Err(invalid(format!("champ inconnu : {k}")));
            }
        }
        let projects: Vec<String> = self.projects.iter().map(|x| x.id.clone()).collect();
        let new_status = match p.get("status") {
            Some(v) => Some(
                v.as_str()
                    .and_then(Status::parse)
                    .ok_or_else(|| invalid("status : todo, doing ou done"))?,
            ),
            None => None,
        };
        let end_order = new_status.map(|s| self.last_order(s) + 1.0);
        let task = self.task_mut(id)?;
        if let Some(v) = p.get("title") {
            task.title = check_title(v.as_str().ok_or_else(|| invalid("title : texte attendu"))?)?;
        }
        if let Some(v) = p.get("notes") {
            task.notes = check_notes(v.as_str().ok_or_else(|| invalid("notes : texte attendu"))?)?;
        }
        if let Some(v) = p.get("project_id") {
            task.project_id = match v {
                Value::Null => None,
                Value::String(s) if projects.contains(s) => Some(s.clone()),
                _ => return Err(Refusal::NotFound("projet de tâches introuvable".into())),
            };
        }
        if let Some(v) = p.get("due") {
            task.due = match v {
                Value::Null => None,
                Value::String(s) if s.trim().is_empty() => None,
                Value::String(s) => Some(check_due(s)?),
                _ => return Err(invalid("due : AAAA-MM-JJ ou null")),
            };
        }
        if let Some(s) = new_status
            && s != task.status
        {
            task.status = s;
            task.done_at = (s == Status::Done).then(now);
            // Changement de colonne sans rang donné : en bas de la colonne.
            if !p.contains_key("order") {
                task.order = end_order.unwrap_or(0.0);
            }
        }
        if let Some(v) = p.get("order") {
            task.order = v
                .as_f64()
                .filter(|o| o.is_finite())
                .ok_or_else(|| invalid("order : nombre attendu"))?;
        }
        task.updated_at = now();
        Ok(task.clone())
    }

    pub fn delete_task(&mut self, id: &str) -> Result<(), Refusal> {
        let n = self.tasks.len();
        self.tasks.retain(|t| t.id != id);
        if self.tasks.len() == n {
            return Err(Refusal::NotFound(format!("tâche {id} introuvable")));
        }
        Ok(())
    }

    /// Supprime un projet ; ses tâches restent, sans projet.
    pub fn delete_project(&mut self, id: &str) -> Result<(), Refusal> {
        let n = self.projects.len();
        self.projects.retain(|p| p.id != id);
        if self.projects.len() == n {
            return Err(Refusal::NotFound("projet de tâches introuvable".into()));
        }
        for t in &mut self.tasks {
            if t.project_id.as_deref() == Some(id) {
                t.project_id = None;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewTask {
    pub title: String,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub status: Option<Status>,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub due: Option<String>,
}

// ---- API de l'UI ----

pub async fn list(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(serde_json::to_value(read(&state).await?)?))
}

pub async fn create(State(state): State<AppState>, ApiJson(b): ApiJson<NewTask>) -> Result<Json<Value>, ApiError> {
    let t = update(&state, |board| board.add_task(b, "michael")).await?;
    Ok(Json(serde_json::to_value(t)?))
}

pub async fn patch(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
    ApiJson(b): ApiJson<Map<String, Value>>,
) -> Result<Json<Value>, ApiError> {
    let t = update(&state, |board| board.patch_task(&id, &b)).await?;
    Ok(Json(serde_json::to_value(t)?))
}

pub async fn delete(State(state): State<AppState>, UrlPath(id): UrlPath<String>) -> Result<Json<Value>, ApiError> {
    update(&state, |board| board.delete_task(&id)).await?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewProject {
    pub name: String,
}

pub async fn create_project(
    State(state): State<AppState>,
    ApiJson(b): ApiJson<NewProject>,
) -> Result<Json<Value>, ApiError> {
    let id = update(&state, |board| board.add_project(&b.name)).await?;
    Ok(Json(json!({"id": id, "name": b.name.trim()})))
}

pub async fn delete_project(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> Result<Json<Value>, ApiError> {
    update(&state, |board| board.delete_project(&id)).await?;
    Ok(Json(json!({"ok": true})))
}

/// Une ligne par tâche pour le modèle.
pub fn line(b: &Board, t: &Task) -> String {
    let mut s = format!("[{}] ({}) {}", t.id, t.status.label(), t.title);
    if let Some(p) = b.project_name(t.project_id.as_deref()) {
        s.push_str(&format!(" · projet {p}"));
    }
    if let Some(d) = &t.due {
        s.push_str(&format!(" · échéance {d}"));
    }
    if !t.notes.is_empty() {
        let first: String = t.notes.lines().next().unwrap_or("").chars().take(120).collect();
        s.push_str(&format!(" · notes : {first}"));
    }
    s
}
