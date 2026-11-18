//! Sessions : historique stocké par EngineX (éléments terminés, masqués),
//! routes `/api/sessions`, flux SSE vers l'UI.

use std::{convert::Infallible, time::Duration};

use axum::{
    Json,
    body::{Body, Bytes},
    extract::{Path as UrlPath, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{broadcast::error::RecvError, mpsc};
use tokio_stream::wrappers::ReceiverStream;

use crate::{
    attachments, bridge,
    error::{ApiError, ApiJson},
    presets, sandboxes,
    state::{AppState, now},
    v1, workspace,
};

#[derive(sqlx::FromRow, Clone, Debug)]
pub struct SessionRow {
    pub id: String,
    pub topic_id: i64,
    pub title: String,
    pub thread_id: Option<String>,
    pub approval: String,
    pub archived: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

const COLS: &str = "id, topic_id, title, thread_id, approval, archived, created_at, updated_at";
/// id, statut, erreur, usage, début, fin.
type TurnRow = (String, String, Option<String>, Option<String>, i64, Option<i64>);

pub const DEFAULT_TITLE: &str = "Nouvelle session";

pub async fn row(state: &AppState, id: &str) -> Result<SessionRow, ApiError> {
    sqlx::query_as(&format!("SELECT {COLS} FROM chat_sessions WHERE id = ?"))
        .bind(id)
        .fetch_optional(state.db())
        .await?
        .ok_or(ApiError::NotFound)
}

pub async fn approval_of(state: &AppState, id: &str) -> Result<String, ApiError> {
    Ok(row(state, id).await?.approval)
}

pub async fn item_count(state: &AppState, id: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT COUNT(*) FROM chat_items WHERE session_id = ?")
        .bind(id)
        .fetch_one(state.db())
        .await
}

/// Stocke un élément terminé (ignoré s'il l'est déjà) ; `true` si ajouté.
pub async fn store_item(
    state: &AppState,
    sid: &str,
    turn_id: Option<&str>,
    item_id: &str,
    kind: &str,
    data: &Value,
) -> Result<bool, sqlx::Error> {
    let r = sqlx::query(
        "INSERT INTO chat_items (session_id, turn_id, item_id, seq, kind, data) VALUES (?, ?, ?, \
         (SELECT COALESCE(MAX(seq), 0) + 1 FROM chat_items WHERE session_id = ?), ?, ?) \
         ON CONFLICT(session_id, item_id) DO NOTHING",
    )
    .bind(sid)
    .bind(turn_id)
    .bind(item_id)
    .bind(sid)
    .bind(kind)
    .bind(data.to_string())
    .execute(state.db())
    .await?;
    Ok(r.rows_affected() == 1)
}

pub async fn set_item_turn(state: &AppState, sid: &str, item_id: &str, turn_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE chat_items SET turn_id = ? WHERE session_id = ? AND item_id = ?")
        .bind(turn_id)
        .bind(sid)
        .bind(item_id)
        .execute(state.db())
        .await
        .map(|_| ())
}

pub async fn insert_turn(state: &AppState, sid: &str, tid: &str, started: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO chat_turns (id, session_id, status, started_at) VALUES (?, ?, 'inProgress', ?) \
         ON CONFLICT(session_id, id) DO NOTHING",
    )
    .bind(tid)
    .bind(sid)
    .bind(started)
    .execute(state.db())
    .await
    .map(|_| ())
}

/// Tour rattrapé (`thread/turns/list`) inconnu d'EngineX.
pub async fn insert_turn_status(state: &AppState, sid: &str, tid: &str, status: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO chat_turns (id, session_id, status, started_at, completed_at) VALUES (?, ?, ?, ?, ?) \
         ON CONFLICT(session_id, id) DO NOTHING",
    )
    .bind(tid)
    .bind(sid)
    .bind(status)
    .bind(now())
    .bind(now())
    .execute(state.db())
    .await
    .map(|_| ())
}

pub async fn finish_turn(
    state: &AppState,
    sid: &str,
    tid: &str,
    started: i64,
    status: &str,
    error: Option<&str>,
    usage: Option<&Value>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO chat_turns (id, session_id, status, error, usage, started_at, completed_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(session_id, id) DO UPDATE SET status = excluded.status, \
         error = excluded.error, usage = excluded.usage, completed_at = excluded.completed_at",
    )
    .bind(tid)
    .bind(sid)
    .bind(status)
    .bind(error)
    .bind(usage.map(Value::to_string))
    .bind(started)
    .bind(now())
    .execute(state.db())
    .await
    .map(|_| ())
}

pub async fn set_thread(state: &AppState, sid: &str, thread: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE chat_sessions SET thread_id = ? WHERE id = ?")
        .bind(thread)
        .bind(sid)
        .execute(state.db())
        .await
        .map(|_| ())
}

pub async fn set_title(state: &AppState, sid: &str, title: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE chat_sessions SET title = ? WHERE id = ? AND title = ?")
        .bind(title)
        .bind(sid)
        .bind(DEFAULT_TITLE)
        .execute(state.db())
        .await
        .map(|_| ())
}

pub async fn touch(state: &AppState, sid: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE chat_sessions SET updated_at = ? WHERE id = ?")
        .bind(now())
        .bind(sid)
        .execute(state.db())
        .await
        .map(|_| ())
}

fn session_json(state: &AppState, r: &SessionRow) -> Value {
    let running = state
        .0
        .hubs
        .lock()
        .expect("verrou")
        .get(&r.id)
        .is_some_and(|h| h.running());
    bridge::session_json(r, running)
}

// ---- Routes ----

#[derive(Deserialize)]
pub struct CreateBody {
    topic_id: i64,
    approval: Option<String>,
}

fn valid_approval(a: &str) -> bool {
    a == "auto" || a == "prudent"
}

pub async fn create(State(state): State<AppState>, ApiJson(b): ApiJson<CreateBody>) -> Result<Json<Value>, ApiError> {
    workspace::topic(&state, b.topic_id).await?;
    let approval = b.approval.unwrap_or_else(|| "auto".into());
    if !valid_approval(&approval) {
        return Err(ApiError::BadRequest);
    }
    let id = uuid::Uuid::new_v4().to_string();
    let t = now();
    sqlx::query(
        "INSERT INTO chat_sessions (id, topic_id, title, approval, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(b.topic_id)
    .bind(DEFAULT_TITLE)
    .bind(&approval)
    .bind(t)
    .bind(t)
    .execute(state.db())
    .await?;
    Ok(Json(json!({ "id": id })))
}

#[derive(Deserialize)]
pub struct ListQuery {
    topic: Option<i64>,
    q: Option<String>,
    archived: Option<String>,
}

pub async fn list(State(state): State<AppState>, Query(q): Query<ListQuery>) -> Result<Json<Value>, ApiError> {
    let archived = matches!(q.archived.as_deref(), Some("1" | "true"));
    let like =
        q.q.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| format!("%{}%", s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")));
    let sql = format!(
        "SELECT {COLS} FROM chat_sessions s WHERE s.archived = ? AND (? IS NULL OR s.topic_id = ?) \
         AND (? IS NULL OR s.title LIKE ? ESCAPE '\\' OR EXISTS (SELECT 1 FROM chat_items i WHERE \
         i.session_id = s.id AND i.kind IN ('userMessage', 'agentMessage') AND i.data LIKE ? ESCAPE '\\')) \
         ORDER BY s.updated_at DESC, s.created_at DESC LIMIT 50"
    );
    let rows: Vec<SessionRow> = sqlx::query_as(&sql)
        .bind(archived as i64)
        .bind(q.topic)
        .bind(q.topic)
        .bind(&like)
        .bind(&like)
        .bind(&like)
        .fetch_all(state.db())
        .await?;
    let sessions: Vec<Value> = rows.iter().map(|r| session_json(&state, r)).collect();
    Ok(Json(json!({ "sessions": sessions })))
}

#[derive(Deserialize)]
pub struct GetQuery {
    before: Option<i64>,
    limit: Option<i64>,
}

pub async fn get(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
    Query(q): Query<GetQuery>,
) -> Result<Json<Value>, ApiError> {
    let r = row(&state, &id).await?;
    let limit = q.limit.unwrap_or(200).clamp(1, 500);
    let before = q.before.unwrap_or(i64::MAX);
    let mut items: Vec<(Option<String>, String, i64, String, String)> = sqlx::query_as(
        "SELECT turn_id, item_id, seq, kind, data FROM chat_items WHERE session_id = ? AND seq < ? \
         ORDER BY seq DESC LIMIT ?",
    )
    .bind(&id)
    .bind(before)
    .bind(limit + 1)
    .fetch_all(state.db())
    .await?;
    let more = items.len() as i64 > limit;
    items.truncate(limit as usize);
    items.reverse();
    let turns: Vec<TurnRow> = sqlx::query_as(
        "SELECT id, status, error, usage, started_at, completed_at FROM chat_turns WHERE session_id = ? ORDER BY started_at",
    )
    .bind(&id)
    .fetch_all(state.db())
    .await?;
    let turns: Vec<Value> = turns
        .into_iter()
        .map(|(tid, st, err, usage, s, c)| {
            json!({"id": tid, "status": st, "error": err,
                "usage": usage.and_then(|u| serde_json::from_str::<Value>(&u).ok()),
                "started_at": s, "completed_at": c})
        })
        .collect();
    let items: Vec<Value> = items
        .into_iter()
        .map(|(tid, iid, seq, kind, data)| {
            json!({"turn_id": tid, "item_id": iid, "seq": seq, "kind": kind,
                "data": serde_json::from_str::<Value>(&data).unwrap_or(Value::Null)})
        })
        .collect();
    Ok(Json(
        json!({"session": session_json(&state, &r), "turns": turns, "items": items, "more": more}),
    ))
}

#[derive(Deserialize)]
pub struct PatchBody {
    title: Option<String>,
    approval: Option<String>,
    archived: Option<bool>,
}

pub async fn patch(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
    ApiJson(b): ApiJson<PatchBody>,
) -> Result<Json<Value>, ApiError> {
    row(&state, &id).await?;
    if let Some(t) = &b.title {
        let t = t.trim();
        if t.is_empty() || t.chars().count() > 200 {
            return Err(ApiError::BadRequest);
        }
        sqlx::query("UPDATE chat_sessions SET title = ? WHERE id = ?")
            .bind(t)
            .bind(&id)
            .execute(state.db())
            .await?;
    }
    if let Some(a) = &b.approval {
        if !valid_approval(a) {
            return Err(ApiError::BadRequest);
        }
        sqlx::query("UPDATE chat_sessions SET approval = ? WHERE id = ?")
            .bind(a)
            .bind(&id)
            .execute(state.db())
            .await?;
    }
    if let Some(a) = b.archived {
        sqlx::query("UPDATE chat_sessions SET archived = ? WHERE id = ?")
            .bind(a as i64)
            .bind(&id)
            .execute(state.db())
            .await?;
        if a {
            bridge::hub(&state, &id).stop_conn(&state, "session archivée").await;
        }
    }
    Ok(Json(json!({ "ok": true })))
}

pub async fn delete(State(state): State<AppState>, UrlPath(id): UrlPath<String>) -> Result<Json<Value>, ApiError> {
    row(&state, &id).await?;
    bridge::hub(&state, &id).stop_conn(&state, "session supprimée").await;
    bridge::forget(&state, &id);
    sqlx::query("DELETE FROM chat_sessions WHERE id = ?")
        .bind(&id)
        .execute(state.db())
        .await?;
    let dir = sandboxes::session_dir(&state, &id);
    if dir.exists()
        && let Err(e) = std::fs::remove_dir_all(&dir)
    {
        tracing::warn!("suppression de {} : {e}", dir.display());
    }
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct TurnBody {
    #[serde(default)]
    text: String,
    #[serde(default)]
    attachments: Vec<String>,
}

pub async fn post_turn(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
    ApiJson(b): ApiJson<TurnBody>,
) -> Result<Response, ApiError> {
    let r = row(&state, &id).await?;
    let text = b.text.trim().to_string();
    if (text.is_empty() && b.attachments.is_empty()) || text.len() > 256 * 1024 || b.attachments.len() > 10 {
        return Err(ApiError::BadRequest);
    }
    let hub = bridge::hub(&state, &id);
    if hub.running() {
        return Err(ApiError::TurnInProgress);
    }
    state.0.sandboxes.backend()?;
    presets::active_id(&state).await?.ok_or(ApiError::NoActivePreset)?;
    let active = v1::active(&state)
        .await
        .map_err(|e| ApiError::Internal(anyhow::anyhow!(e.message)))?
        .ok_or(ApiError::NoActivePreset)?;
    if v1::not_ready(&state.supervisor().state()).is_some() {
        return Err(ApiError::EngineNotReady);
    }
    if !b.attachments.is_empty() && !active.vision {
        return Err(ApiError::NoVision);
    }
    for a in &b.attachments {
        if !attachments::exists(&state, &id, a) {
            return Err(ApiError::BadRequest);
        }
    }
    if !hub.has_conn() {
        make_room(&state, &id).await?;
    }
    let masker = state.masker();
    let images: Vec<Value> = b
        .attachments
        .iter()
        .map(|a| json!({"name": a, "url": attachments::url(&id, a)}))
        .collect();
    let user = json!({
        "type": "userMessage",
        "id": format!("ex-{}", uuid::Uuid::new_v4()),
        "text": masker.mask(&text),
        "images": images,
    });
    hub.begin(user)?;
    let _ = r;
    tokio::spawn(hub.run_turn(state.clone(), text, b.attachments));
    Ok((StatusCode::ACCEPTED, Json(json!({ "accepted": true }))).into_response())
}

/// Limite `sandbox.max` : la plus ancienne sandbox inactive est arrêtée,
/// sinon `too_many_sandboxes`.
async fn make_room(state: &AppState, for_id: &str) -> Result<(), ApiError> {
    let max = sandboxes::max_live(state).await as usize;
    let live: Vec<_> = bridge::hubs(state)
        .into_iter()
        .filter(|h| h.id != for_id && h.has_conn())
        .collect();
    if live.len() < max {
        return Ok(());
    }
    let oldest = live
        .iter()
        .filter_map(|h| h.idle_for().map(|d| (d, h.clone())))
        .max_by_key(|(d, _)| *d);
    match oldest {
        Some((_, h)) => {
            tracing::info!("limite de sandboxes : {} arrêtée", h.id);
            h.stop_conn(state, "place libérée pour une autre session").await;
            Ok(())
        }
        None => Err(ApiError::TooManySandboxes),
    }
}

pub async fn interrupt(State(state): State<AppState>, UrlPath(id): UrlPath<String>) -> Result<Json<Value>, ApiError> {
    row(&state, &id).await?;
    bridge::hub(&state, &id).interrupt().await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct AnswerBody {
    decision: Option<String>,
    answers: Option<Value>,
}

pub async fn answer(
    State(state): State<AppState>,
    UrlPath((id, rid)): UrlPath<(String, String)>,
    ApiJson(b): ApiJson<AnswerBody>,
) -> Result<Json<Value>, ApiError> {
    row(&state, &id).await?;
    let result = match (b.decision, b.answers) {
        (Some(d), None) if ["accept", "acceptForSession", "decline", "cancel"].contains(&d.as_str()) => {
            json!({ "decision": d })
        }
        (None, Some(a @ Value::Object(_))) => json!({ "answers": a }),
        _ => return Err(ApiError::BadRequest),
    };
    bridge::hub(&state, &id).answer(&rid, result)?;
    Ok(Json(json!({ "ok": true })))
}

fn sse(name: &str, data: &Value) -> Bytes {
    Bytes::from(format!("event: {name}\ndata: {data}\n\n"))
}

pub async fn events(State(state): State<AppState>, UrlPath(id): UrlPath<String>) -> Result<Response, ApiError> {
    row(&state, &id).await?;
    let hub = bridge::hub(&state, &id);
    let mut rx = hub.subscribe();
    let (tx, out) = mpsc::channel::<Result<Bytes, Infallible>>(256);
    let keepalive = state.config().timings.keepalive.max(Duration::from_millis(100));
    tokio::spawn(async move {
        let snapshot = |state: AppState, hub: std::sync::Arc<bridge::Hub>, id: String| async move {
            let s = row(&state, &id)
                .await
                .ok()
                .map(|r| session_json(&state, &r))
                .unwrap_or(Value::Null);
            hub.snapshot(s)
        };
        let snap = snapshot(state.clone(), hub.clone(), id.clone()).await;
        if tx.send(Ok(sse("snapshot", &snap))).await.is_err() {
            return;
        }
        loop {
            tokio::select! {
                ev = rx.recv() => match ev {
                    Ok(ev) => {
                        if tx.send(Ok(sse(ev.name, &ev.data))).await.is_err() {
                            return;
                        }
                    }
                    Err(RecvError::Lagged(_)) => {
                        let snap = snapshot(state.clone(), hub.clone(), id.clone()).await;
                        if tx.send(Ok(sse("snapshot", &snap))).await.is_err() {
                            return;
                        }
                    }
                    Err(RecvError::Closed) => return,
                },
                _ = tokio::time::sleep(keepalive) => {
                    if tx.send(Ok(Bytes::from_static(b": ping\n\n"))).await.is_err() {
                        return;
                    }
                }
                _ = tx.closed() => return,
            }
        }
    });
    Ok(Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        .header("x-accel-buffering", "no")
        .body(Body::from_stream(ReceiverStream::new(out)))
        .expect("réponse SSE"))
}
