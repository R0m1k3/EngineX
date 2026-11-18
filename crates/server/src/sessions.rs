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
    presets, sandboxes, snapshots,
    state::{AppState, now},
    v1, verify, workspace,
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
    pub copy_branch: Option<String>,
    pub pending_note: Option<String>,
    pub pending_revert: Option<String>,
    pub undo_snap: Option<String>,
}

const COLS: &str = "id, topic_id, title, thread_id, approval, archived, created_at, updated_at, copy_branch, \
                    pending_note, pending_revert, undo_snap";
/// id, statut, erreur, usage, début, fin, origine, pre, post, modifications.
type TurnRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    i64,
    Option<i64>,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

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

pub async fn insert_turn(
    state: &AppState,
    sid: &str,
    tid: &str,
    started: i64,
    origin: &str,
    snap_pre: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO chat_turns (id, session_id, status, started_at, origin, snap_pre) \
         VALUES (?, ?, 'inProgress', ?, ?, ?) ON CONFLICT(session_id, id) DO NOTHING",
    )
    .bind(tid)
    .bind(sid)
    .bind(started)
    .bind(origin)
    .bind(snap_pre)
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

/// Instantanés et modifications d'un tour (après son `post`).
pub async fn set_turn_snaps(
    state: &AppState,
    sid: &str,
    tid: &str,
    pre: Option<&str>,
    post: Option<&str>,
    changes: Option<&Value>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE chat_turns SET snap_pre = COALESCE(?, snap_pre), snap_post = ?, changes = ? \
         WHERE session_id = ? AND id = ?",
    )
    .bind(pre)
    .bind(post)
    .bind(changes.map(Value::to_string))
    .bind(sid)
    .bind(tid)
    .execute(state.db())
    .await
    .map(|_| ())
}

/// Colonne texte facultative de `chat_sessions` (`pending_note`,
/// `pending_revert`, `undo_snap`).
pub async fn set_session_field(state: &AppState, sid: &str, field: &str, v: Option<&str>) -> Result<(), sqlx::Error> {
    let col = match field {
        "pending_note" => "pending_note",
        "pending_revert" => "pending_revert",
        "undo_snap" => "undo_snap",
        _ => return Ok(()),
    };
    sqlx::query(&format!("UPDATE chat_sessions SET {col} = ? WHERE id = ?"))
        .bind(v)
        .bind(sid)
        .execute(state.db())
        .await
        .map(|_| ())
}

/// Tours retirés par un retour de conversation (jamais restockés).
pub async fn reverted(state: &AppState, sid: &str) -> Result<std::collections::HashSet<String>, sqlx::Error> {
    let v: Vec<(String,)> = sqlx::query_as("SELECT turn_id FROM chat_reverted WHERE session_id = ?")
        .bind(sid)
        .fetch_all(state.db())
        .await?;
    Ok(v.into_iter().map(|(t,)| t).collect())
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
        "SELECT id, status, error, usage, started_at, completed_at, origin, snap_pre, snap_post, changes \
         FROM chat_turns WHERE session_id = ? ORDER BY started_at, rowid",
    )
    .bind(&id)
    .fetch_all(state.db())
    .await?;
    let mut verifs = verify::by_turn(&state, &id).await?;
    let green: Option<String> = sqlx::query_scalar(
        "SELECT turn_id FROM verifications WHERE session_id = ? AND status = 'passed' ORDER BY id DESC LIMIT 1",
    )
    .bind(&id)
    .fetch_optional(state.db())
    .await?;
    let turns: Vec<Value> = turns
        .into_iter()
        .map(|(tid, st, err, usage, s, c, origin, pre, post, changes)| {
            let v = verifs.remove(&tid).unwrap_or_default();
            json!({"id": tid, "status": st, "error": err,
                "usage": usage.and_then(|u| serde_json::from_str::<Value>(&u).ok()),
                "started_at": s, "completed_at": c, "origin": origin,
                "has_snapshot": pre.is_some(), "has_post": post.is_some(),
                "changes": changes.and_then(|c| serde_json::from_str::<Value>(&c).ok()),
                "verifications": v})
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
        json!({"session": session_json(&state, &r), "turns": turns, "items": items, "more": more,
        "green_turn": green}),
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
    let r = row(&state, &id).await?;
    if let Ok(f) = snapshots::folder(&state, &r).await {
        if r.copy_branch.is_some() {
            let _ = std::fs::remove_dir_all(&f.snap);
        } else if let Err(e) = snapshots::drop_refs(&state, &f).await {
            tracing::warn!("refs de la session {id} : {e:#}");
        }
    }
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
    if hub.verifying() {
        hub.cancel_verify().await;
        for _ in 0..50 {
            if !hub.verifying() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
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
    hub.begin(user, "user")?;
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
    let hub = bridge::hub(&state, &id);
    if hub.verifying() && !hub.running() {
        hub.cancel_verify().await;
        return Ok(Json(json!({ "ok": true })));
    }
    hub.interrupt().await?;
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

// ---- Instantanés et retours arrière ----

/// 409 `folder_busy` si un tour ou une vérification tourne dans une session
/// qui partage ce dossier de travail.
async fn ensure_folder_free(state: &AppState, f: &snapshots::Folder) -> Result<(), ApiError> {
    let want = std::fs::canonicalize(&f.work).unwrap_or_else(|_| f.work.clone());
    for h in bridge::hubs(state) {
        if !h.busy() {
            continue;
        }
        let Ok(r) = row(state, &h.id).await else { continue };
        let Ok(o) = snapshots::folder(state, &r).await else {
            continue;
        };
        if std::fs::canonicalize(&o.work).unwrap_or(o.work) == want {
            return Err(ApiError::FolderBusy);
        }
    }
    Ok(())
}

/// `(snap_pre, snap_post)` d'un tour.
async fn turn_snaps(state: &AppState, sid: &str, tid: &str) -> Result<(Option<String>, Option<String>), ApiError> {
    sqlx::query_as("SELECT snap_pre, snap_post FROM chat_turns WHERE session_id = ? AND id = ?")
        .bind(sid)
        .bind(tid)
        .fetch_optional(state.db())
        .await?
        .ok_or(ApiError::NotFound)
}

fn internal(e: anyhow::Error) -> ApiError {
    ApiError::Internal(e)
}

#[derive(Deserialize)]
pub struct DiffQuery {
    path: String,
}

pub async fn turn_diff(
    State(state): State<AppState>,
    UrlPath((id, tid)): UrlPath<(String, String)>,
    Query(q): Query<DiffQuery>,
) -> Result<Json<Value>, ApiError> {
    let r = row(&state, &id).await?;
    if !snapshots::valid_rel(&q.path) {
        return Err(ApiError::BadRequest);
    }
    let (Some(pre), Some(post)) = turn_snaps(&state, &id, &tid).await? else {
        return Err(ApiError::NoSnapshot);
    };
    let f = snapshots::folder(&state, &r).await?;
    let (diff, truncated, binary) = snapshots::diff(&state, &f, &pre, &post, &q.path)
        .await
        .map_err(internal)?;
    Ok(Json(json!({"diff": diff, "truncated": truncated, "binary": binary})))
}

#[derive(Deserialize)]
pub struct UndoFileBody {
    path: String,
    #[serde(default)]
    force: bool,
}

pub async fn undo_file(
    State(state): State<AppState>,
    UrlPath((id, tid)): UrlPath<(String, String)>,
    ApiJson(b): ApiJson<UndoFileBody>,
) -> Result<Json<Value>, ApiError> {
    let r = row(&state, &id).await?;
    if !snapshots::valid_rel(&b.path) {
        return Err(ApiError::BadRequest);
    }
    let (Some(pre), Some(post)) = turn_snaps(&state, &id, &tid).await? else {
        return Err(ApiError::NoSnapshot);
    };
    let f = snapshots::folder(&state, &r).await?;
    ensure_folder_free(&state, &f).await?;
    match snapshots::undo_file(&state, &f, &pre, &post, &b.path, b.force)
        .await
        .map_err(internal)?
    {
        snapshots::UndoFile::ChangedSince => Err(ApiError::ChangedSince),
        snapshots::UndoFile::Done(undo) => {
            set_session_field(&state, &id, "undo_snap", Some(&undo)).await?;
            emit_session(&state, &id).await;
            Ok(Json(json!({ "ok": true })))
        }
    }
}

/// Le détail de la session a changé (bandeau, note) : nouvel instantané SSE.
async fn emit_session(state: &AppState, id: &str) {
    if let Ok(r) = row(state, id).await {
        bridge::hub(state, id).emit_event("session", session_json(state, &r));
    }
}

/// Texte du message de Michael d'un tour.
async fn user_item(state: &AppState, sid: &str, tid: &str) -> Result<Option<Value>, ApiError> {
    let d: Option<String> = sqlx::query_scalar(
        "SELECT data FROM chat_items WHERE session_id = ? AND turn_id = ? AND kind = 'userMessage' ORDER BY seq LIMIT 1",
    )
    .bind(sid)
    .bind(tid)
    .fetch_optional(state.db())
    .await?;
    Ok(d.and_then(|d| serde_json::from_str(&d).ok()))
}

fn note_for(user: Option<&Value>) -> String {
    let text: String = user
        .and_then(|u| u["text"].as_str())
        .unwrap_or("")
        .chars()
        .take(60)
        .collect();
    format!(
        "Note d'EngineX : les fichiers ont été remis dans l'état d'avant le message « {text} ». \
         Vérifie l'état actuel avant de continuer."
    )
}

#[derive(Deserialize)]
pub struct RevertBody {
    #[serde(default)]
    conversation: bool,
}

/// « Revenir avant ce message » : fichiers, et la conversation si demandé.
pub async fn revert(
    State(state): State<AppState>,
    UrlPath((id, tid)): UrlPath<(String, String)>,
    ApiJson(b): ApiJson<RevertBody>,
) -> Result<Json<Value>, ApiError> {
    let r = row(&state, &id).await?;
    let (pre, _) = turn_snaps(&state, &id, &tid).await?;
    let pre = pre.ok_or(ApiError::NoSnapshot)?;
    let f = snapshots::folder(&state, &r).await?;
    ensure_folder_free(&state, &f).await?;
    let undo = snapshots::restore_to(&state, &f, &pre).await.map_err(internal)?;
    set_session_field(&state, &id, "undo_snap", Some(&undo)).await?;
    let user = user_item(&state, &id, &tid).await?;
    if !b.conversation {
        set_session_field(&state, &id, "pending_note", Some(&note_for(user.as_ref()))).await?;
        emit_session(&state, &id).await;
        return Ok(Json(json!({"ok": true, "user": null})));
    }
    // Conversation : ce tour et les suivants quittent l'historique d'EngineX
    // tout de suite, et celui de Codex dès que possible.
    let started: i64 = sqlx::query_scalar("SELECT started_at FROM chat_turns WHERE session_id = ? AND id = ?")
        .bind(&id)
        .bind(&tid)
        .fetch_one(state.db())
        .await?;
    let first_seq: Option<i64> =
        sqlx::query_scalar("SELECT MIN(seq) FROM chat_items WHERE session_id = ? AND turn_id = ?")
            .bind(&id)
            .bind(&tid)
            .fetch_one(state.db())
            .await?;
    let first_seq = first_seq.unwrap_or(i64::MAX);
    let mut removed: Vec<(String, i64)> = sqlx::query_as(
        "SELECT id, started_at FROM chat_turns WHERE session_id = ? AND (started_at > ? OR id = ? OR id IN \
         (SELECT turn_id FROM chat_items WHERE session_id = ? AND seq >= ? AND turn_id IS NOT NULL))",
    )
    .bind(&id)
    .bind(started)
    .bind(&tid)
    .bind(&id)
    .bind(first_seq)
    .fetch_all(state.db())
    .await?;
    removed.sort_by_key(|(t, s)| (*s, t != &tid));
    let mut tx = state.db().begin().await?;
    for (t, _) in &removed {
        sqlx::query("INSERT INTO chat_reverted (session_id, turn_id) VALUES (?, ?) ON CONFLICT DO NOTHING")
            .bind(&id)
            .bind(t)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM chat_turns WHERE session_id = ? AND id = ?")
            .bind(&id)
            .bind(t)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM verifications WHERE session_id = ? AND turn_id = ?")
            .bind(&id)
            .bind(t)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM chat_items WHERE session_id = ? AND turn_id = ?")
            .bind(&id)
            .bind(t)
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("DELETE FROM chat_items WHERE session_id = ? AND seq >= ?")
        .bind(&id)
        .bind(first_seq)
        .execute(&mut *tx)
        .await?;
    // Un tour `ex-…` est inconnu de Codex : le premier tour retiré connu.
    let pending = removed.iter().map(|(t, _)| t).find(|t| !t.starts_with("ex-")).cloned();
    if let Some(p) = &pending {
        sqlx::query("UPDATE chat_sessions SET pending_revert = ? WHERE id = ? AND pending_revert IS NULL")
            .bind(p)
            .bind(&id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    let hub = bridge::hub(&state, &id);
    if pending.is_some() && hub.has_conn() {
        let st = state.clone();
        tokio::spawn(async move { hub.revert_now(&st).await });
    }
    emit_session(&state, &id).await;
    bridge::hub(&state, &id).emit_event("history", json!({ "reverted": tid }));
    Ok(Json(json!({"ok": true, "user": user})))
}

/// « Rétablir » : les fichiers d'avant le dernier retour arrière.
pub async fn restore_undo(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> Result<Json<Value>, ApiError> {
    let r = row(&state, &id).await?;
    let undo = r.undo_snap.clone().ok_or(ApiError::NoSnapshot)?;
    let f = snapshots::folder(&state, &r).await?;
    ensure_folder_free(&state, &f).await?;
    let before = snapshots::restore_to(&state, &f, &undo).await.map_err(internal)?;
    set_session_field(&state, &id, "undo_snap", Some(&before)).await?;
    emit_session(&state, &id).await;
    Ok(Json(json!({ "ok": true })))
}

/// « Revenir au dernier état vert » (fichiers seulement).
pub async fn restore_green(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<String>,
) -> Result<Json<Value>, ApiError> {
    let r = row(&state, &id).await?;
    let green: Option<String> = sqlx::query_scalar(
        "SELECT v.turn_id FROM verifications v JOIN chat_turns t ON t.session_id = v.session_id AND t.id = v.turn_id \
         WHERE v.session_id = ? AND v.status = 'passed' AND t.snap_post IS NOT NULL ORDER BY v.id DESC LIMIT 1",
    )
    .bind(&id)
    .fetch_optional(state.db())
    .await?;
    let green = green.ok_or(ApiError::NoSnapshot)?;
    let f = snapshots::folder(&state, &r).await?;
    ensure_folder_free(&state, &f).await?;
    let undo = snapshots::restore_to(&state, &f, &snapshots::green_ref(&id))
        .await
        .map_err(internal)?;
    set_session_field(&state, &id, "undo_snap", Some(&undo)).await?;
    let user = user_item(&state, &id, &green).await?;
    let note = note_for(user.as_ref()).replace(
        "dans l'état d'avant le message",
        "dans le dernier état vérifié (vert), celui d'après le message",
    );
    set_session_field(&state, &id, "pending_note", Some(&note)).await?;
    emit_session(&state, &id).await;
    Ok(Json(json!({ "ok": true })))
}
