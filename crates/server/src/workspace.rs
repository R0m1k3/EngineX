//! Projets et topics : `<data>/workspace/projects/<projet>/topics/<topic>/`.
//! Premier démarrage : projet `general`, topic `inbox`. L'étape 5 y ajoutera
//! AGENTS.md, memory.md et les skills.

use std::path::PathBuf;

use axum::{
    Json,
    extract::{Path as UrlPath, State},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    error::{ApiError, ApiJson},
    state::{AppState, now},
};

const NAME_MAX: usize = 60;
const SLUG_MAX: usize = 40;

/// Slug `[a-z0-9-]{1,40}` dérivé du nom (accents retirés).
pub fn slugify(name: &str) -> Option<String> {
    let mut out = String::new();
    for c in name.trim().chars() {
        let c = match c {
            'à' | 'â' | 'ä' | 'á' | 'ã' | 'À' | 'Â' | 'Ä' | 'Á' => 'a',
            'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => 'e',
            'î' | 'ï' | 'í' | 'Î' | 'Ï' => 'i',
            'ô' | 'ö' | 'ó' | 'õ' | 'Ô' | 'Ö' => 'o',
            'ù' | 'û' | 'ü' | 'ú' | 'Ù' | 'Û' | 'Ü' => 'u',
            'ç' | 'Ç' => 'c',
            'ñ' => 'n',
            c => c.to_ascii_lowercase(),
        };
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let out: String = out.trim_matches('-').chars().take(SLUG_MAX).collect();
    let out = out.trim_end_matches('-').to_string();
    (!out.is_empty()).then_some(out)
}

fn clean_name(name: &str) -> Result<(String, String), ApiError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > NAME_MAX {
        return Err(ApiError::InvalidName);
    }
    let slug = slugify(name).ok_or(ApiError::InvalidName)?;
    Ok((name.to_string(), slug))
}

pub fn project_dir(state: &AppState, project: &str) -> PathBuf {
    state.config().data_dir.join("workspace/projects").join(project)
}

pub fn topic_path(state: &AppState, project: &str, topic: &str) -> PathBuf {
    project_dir(state, project).join("topics").join(topic)
}

/// Topic d'une session : dossier, slug et id du projet.
pub struct TopicInfo {
    pub id: i64,
    pub project_id: i64,
    pub project_slug: String,
    pub dir: PathBuf,
}

pub async fn topic(state: &AppState, topic_id: i64) -> Result<TopicInfo, ApiError> {
    let (pid, pslug, tslug): (i64, String, String) = sqlx::query_as(
        "SELECT p.id, p.slug, t.slug FROM topics t JOIN projects p ON p.id = t.project_id WHERE t.id = ?",
    )
    .bind(topic_id)
    .fetch_optional(state.db())
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(TopicInfo {
        id: topic_id,
        project_id: pid,
        dir: topic_path(state, &pslug, &tslug),
        project_slug: pslug,
    })
}

/// Crée le dossier et vérifie qu'il est inscriptible (erreur avec le chemin).
fn make_dir(dir: &std::path::Path) -> Result<(), ApiError> {
    let probe = dir.join(".enginex-write-test");
    std::fs::create_dir_all(dir)
        .and_then(|_| std::fs::write(&probe, b"ok"))
        .and_then(|_| std::fs::remove_file(&probe))
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("{} non inscriptible : {e}", dir.display())))
}

async fn insert_project(state: &AppState, name: &str, slug: &str) -> Result<i64, ApiError> {
    let r: Result<i64, sqlx::Error> =
        sqlx::query_scalar("INSERT INTO projects (slug, name, created_at) VALUES (?, ?, ?) RETURNING id")
            .bind(slug)
            .bind(name)
            .bind(now())
            .fetch_one(state.db())
            .await;
    match r {
        Ok(id) => {
            make_dir(&project_dir(state, slug))?;
            Ok(id)
        }
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => Err(ApiError::NameTaken),
        Err(e) => Err(e.into()),
    }
}

async fn insert_topic(state: &AppState, project_id: i64, name: &str, slug: &str) -> Result<i64, ApiError> {
    let pslug: String = sqlx::query_scalar("SELECT slug FROM projects WHERE id = ?")
        .bind(project_id)
        .fetch_optional(state.db())
        .await?
        .ok_or(ApiError::NotFound)?;
    let dir = topic_path(state, &pslug, slug);
    make_dir(&dir)?;
    let r: Result<i64, sqlx::Error> =
        sqlx::query_scalar("INSERT INTO topics (project_id, slug, name, created_at) VALUES (?, ?, ?, ?) RETURNING id")
            .bind(project_id)
            .bind(slug)
            .bind(name)
            .bind(now())
            .fetch_one(state.db())
            .await;
    match r {
        Ok(id) => Ok(id),
        Err(sqlx::Error::Database(e)) if e.is_unique_violation() => Err(ApiError::NameTaken),
        Err(e) => Err(e.into()),
    }
}

/// Premier démarrage : `general/inbox`.
pub async fn boot(state: &AppState) -> anyhow::Result<()> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM projects")
        .fetch_one(state.db())
        .await?;
    if n == 0 {
        let pid = insert_project(state, "general", "general")
            .await
            .map_err(|e| anyhow::anyhow!("projet general : {e:?}"))?;
        insert_topic(state, pid, "inbox", "inbox")
            .await
            .map_err(|e| anyhow::anyhow!("topic inbox : {e:?}"))?;
    }
    Ok(())
}

// ---- Routes ----

pub async fn list(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let projects: Vec<(i64, String, String)> = sqlx::query_as("SELECT id, slug, name FROM projects ORDER BY name, id")
        .fetch_all(state.db())
        .await?;
    let topics: Vec<(i64, i64, String, String, i64)> = sqlx::query_as(
        "SELECT t.id, t.project_id, t.slug, t.name, \
         (SELECT COUNT(*) FROM chat_sessions s WHERE s.topic_id = t.id AND s.archived = 0) \
         FROM topics t ORDER BY t.name, t.id",
    )
    .fetch_all(state.db())
    .await?;
    let out: Vec<Value> = projects
        .into_iter()
        .map(|(id, slug, name)| {
            let t: Vec<Value> = topics
                .iter()
                .filter(|t| t.1 == id)
                .map(|(tid, _, ts, tn, n)| json!({"id": tid, "slug": ts, "name": tn, "sessions": n}))
                .collect();
            json!({"id": id, "slug": slug, "name": name, "topics": t})
        })
        .collect();
    Ok(Json(json!({ "projects": out })))
}

#[derive(Deserialize)]
pub struct NameBody {
    name: String,
}

pub async fn create_project(
    State(state): State<AppState>,
    ApiJson(b): ApiJson<NameBody>,
) -> Result<Json<Value>, ApiError> {
    let (name, slug) = clean_name(&b.name)?;
    let id = insert_project(&state, &name, &slug).await?;
    Ok(Json(json!({"id": id, "slug": slug})))
}

pub async fn create_topic(
    State(state): State<AppState>,
    UrlPath(project_id): UrlPath<i64>,
    ApiJson(b): ApiJson<NameBody>,
) -> Result<Json<Value>, ApiError> {
    let (name, slug) = clean_name(&b.name)?;
    let id = insert_topic(&state, project_id, &name, &slug).await?;
    Ok(Json(json!({"id": id, "slug": slug})))
}

fn dir_empty(dir: &std::path::Path) -> bool {
    match std::fs::read_dir(dir) {
        Ok(mut d) => d.next().is_none(),
        Err(_) => true,
    }
}

/// Topic vide : aucune session (archivées comprises) et dossier vide.
async fn topic_empty(state: &AppState, t: &TopicInfo) -> Result<bool, ApiError> {
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chat_sessions WHERE topic_id = ?")
        .bind(t.id)
        .fetch_one(state.db())
        .await?;
    Ok(n == 0 && dir_empty(&t.dir))
}

pub async fn delete_topic(State(state): State<AppState>, UrlPath(id): UrlPath<i64>) -> Result<Json<Value>, ApiError> {
    let t = topic(&state, id).await?;
    if !topic_empty(&state, &t).await? {
        return Err(ApiError::NotEmpty);
    }
    sqlx::query("DELETE FROM topics WHERE id = ?")
        .bind(id)
        .execute(state.db())
        .await?;
    let _ = std::fs::remove_dir(&t.dir);
    let _ = std::fs::remove_dir_all(crate::snapshots::topic_repo(&state, id));
    Ok(Json(json!({ "ok": true })))
}

/// Projet supprimé seulement si tous ses topics sont vides.
pub async fn delete_project(State(state): State<AppState>, UrlPath(id): UrlPath<i64>) -> Result<Json<Value>, ApiError> {
    let slug: String = sqlx::query_scalar("SELECT slug FROM projects WHERE id = ?")
        .bind(id)
        .fetch_optional(state.db())
        .await?
        .ok_or(ApiError::NotFound)?;
    let tids: Vec<i64> = sqlx::query_scalar("SELECT id FROM topics WHERE project_id = ?")
        .bind(id)
        .fetch_all(state.db())
        .await?;
    let mut topics = Vec::new();
    for tid in tids {
        let t = topic(&state, tid).await?;
        if !topic_empty(&state, &t).await? {
            return Err(ApiError::NotEmpty);
        }
        topics.push(t);
    }
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM secrets WHERE scope = ?")
        .bind(format!("project:{id}"))
        .fetch_one(state.db())
        .await?;
    if n > 0 {
        return Err(ApiError::NotEmpty);
    }
    sqlx::query("DELETE FROM topics WHERE project_id = ?")
        .bind(id)
        .execute(state.db())
        .await?;
    sqlx::query("DELETE FROM projects WHERE id = ?")
        .bind(id)
        .execute(state.db())
        .await?;
    for t in topics {
        let _ = std::fs::remove_dir(&t.dir);
        let _ = std::fs::remove_dir_all(crate::snapshots::topic_repo(&state, t.id));
    }
    let dir = project_dir(&state, &slug);
    let _ = std::fs::remove_dir(dir.join("topics"));
    let _ = std::fs::remove_dir(&dir);
    Ok(Json(json!({ "ok": true })))
}
