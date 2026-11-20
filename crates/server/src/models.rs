//! Hugging Face (recherche, fiche d'un dépôt, verdict de place),
//! file des téléchargements et modèles locaux.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Component, Path, PathBuf},
    sync::{LazyLock, Mutex},
    time::{Duration, Instant},
};

use axum::{
    Json,
    extract::{Path as UrlPath, Query, State},
};
use enginex_hub::{
    download::{self, DownloadError},
    fit::{self, CardVram, FitInput, FitReport, KvType},
    gguf::{self, GgufMeta},
    hf::{FileSpec, RepoInfo},
};
use regex::Regex;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use crate::{
    error::{ApiError, ApiJson},
    state::{AppState, now},
    system::setting,
};

static REPO: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]*/[A-Za-z0-9._-]+$").expect("motif"));

const SEARCH_TTL: Duration = Duration::from_secs(60);
const REPO_TTL: Duration = Duration::from_secs(600);
const HEADER_TTL: Duration = Duration::from_secs(24 * 3600);
const CACHE_MAX: usize = 500;

pub fn valid_repo(repo: &str) -> bool {
    REPO.is_match(repo) && !repo.contains("..")
}

/// Nom de fichier venu de Hugging Face : relatif, sans `..`, `.gguf`.
pub fn valid_file(name: &str) -> bool {
    let p = Path::new(name);
    name.to_ascii_lowercase().ends_with(".gguf")
        && !name.starts_with('/')
        && p.components().all(|c| matches!(c, Component::Normal(_)))
}

/// Chemin local d'un fichier d'un dépôt, vérifié sous `models_dir`.
pub fn model_path(models_dir: &Path, repo: &str, file: &str) -> Result<PathBuf, ApiError> {
    if !valid_repo(repo) || !valid_file(file) {
        return Err(ApiError::BadRequest);
    }
    let p = models_dir.join(repo).join(file);
    if !p.starts_with(models_dir) {
        return Err(ApiError::BadRequest);
    }
    Ok(p)
}

fn part_of(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

// ---- Cache des réponses Hugging Face ----

fn cache_get(state: &AppState, key: &str, ttl: Duration) -> Option<Value> {
    let c = state.0.hf_cache.lock().expect("cache");
    c.get(key).filter(|(t, _)| t.elapsed() < ttl).map(|(_, v)| v.clone())
}

fn cache_put(state: &AppState, key: String, v: Value) {
    let mut c = state.0.hf_cache.lock().expect("cache");
    if c.len() >= CACHE_MAX {
        c.clear();
    }
    c.insert(key, (Instant::now(), v));
}

pub async fn repo_info(state: &AppState, repo: &str) -> Result<RepoInfo, ApiError> {
    let key = format!("repo:{repo}");
    if let Some(v) = cache_get(state, &key, REPO_TTL) {
        return Ok(serde_json::from_value(v)?);
    }
    let info = state.0.hf.repo(repo).await?;
    cache_put(state, key, serde_json::to_value(&info)?);
    Ok(info)
}

async fn header(state: &AppState, repo: &str, file: &str) -> Result<GgufMeta, ApiError> {
    let key = format!("header:{repo}/{file}");
    if let Some(v) = cache_get(state, &key, HEADER_TTL) {
        return Ok(serde_json::from_value(v)?);
    }
    let meta = state.0.hf.header(repo, file).await?;
    cache_put(state, key, serde_json::to_value(&meta)?);
    Ok(meta)
}

// ---- Routes Hugging Face ----

#[derive(Deserialize)]
pub struct SearchQuery {
    q: String,
}

pub async fn search(State(state): State<AppState>, Query(q): Query<SearchQuery>) -> Result<Json<Value>, ApiError> {
    let text = q.q.trim();
    if text.is_empty() || text.len() > 200 {
        return Err(ApiError::BadRequest);
    }
    let key = format!("search:{text}");
    if let Some(v) = cache_get(&state, &key, SEARCH_TTL) {
        return Ok(Json(v));
    }
    let v = serde_json::to_value(state.0.hf.search(text).await?)?;
    cache_put(&state, key, v.clone());
    Ok(Json(v))
}

#[derive(Deserialize)]
pub struct RepoQuery {
    repo: String,
}

pub async fn repo(State(state): State<AppState>, Query(q): Query<RepoQuery>) -> Result<Json<RepoInfo>, ApiError> {
    if !valid_repo(&q.repo) {
        return Err(ApiError::BadRequest);
    }
    Ok(Json(repo_info(&state, &q.repo).await?))
}

/// VRAM disponible par carte : totale (nvidia-smi) moins la part des autres.
pub async fn cards(state: &AppState) -> (Vec<CardVram>, Option<i64>) {
    let gpus = enginex_engine::probe::read_gpus(&state.config().nvidia_smi)
        .await
        .unwrap_or_default();
    let others = state.0.vram_others.lock().expect("vram").clone();
    let cards = gpus
        .iter()
        .enumerate()
        .map(|(i, g)| CardVram {
            total_mib: g.total_mib.max(0) as u64,
            others_mib: others.as_ref().and_then(|(_, o)| o.get(i).copied()).unwrap_or(0),
        })
        .collect();
    (cards, others.map(|(t, _)| t))
}

#[derive(Deserialize)]
pub struct FitQuery {
    repo: String,
    file: String,
    ctx: Option<u64>,
    kv: Option<String>,
    mmproj: Option<String>,
}

pub async fn fit_route(State(state): State<AppState>, Query(q): Query<FitQuery>) -> Result<Json<Value>, ApiError> {
    if !valid_repo(&q.repo) || !valid_file(&q.file) {
        return Err(ApiError::BadRequest);
    }
    let kv = KvType::parse(q.kv.as_deref().unwrap_or("q8_0")).ok_or(ApiError::BadRequest)?;
    let ctx = q.ctx.unwrap_or(65536);
    if !(512..=1_048_576).contains(&ctx) {
        return Err(ApiError::BadRequest);
    }
    let info = repo_info(&state, &q.repo).await?;
    let group = info
        .groups
        .iter()
        .find(|g| g.files.iter().any(|f| f.name == q.file))
        .ok_or(ApiError::NotFound)?;
    let mmproj_bytes = match &q.mmproj {
        Some(m) => {
            info.mmproj
                .iter()
                .find(|f| &f.name == m)
                .ok_or(ApiError::BadRequest)?
                .size
        }
        None => 0,
    };
    let meta = header(&state, &q.repo, &group.files[0].name).await?;
    let (cards, measured_at) = cards(&state).await;
    let report = fit::estimate(
        &FitInput {
            meta: &meta,
            weights_bytes: group.total_size,
            mmproj_bytes,
            ctx,
            parallel: 1,
            kv,
            fit_target_mib: 1024,
            draft_bytes: 0,
            flash_attn: true,
            ubatch: 0,
            mtp: false,
        },
        &cards,
    );
    Ok(Json(
        json!({ "report": report, "meta": meta, "measured_at": measured_at }),
    ))
}

// ---- File des téléchargements ----

/// File unique : un téléchargement actif à la fois.
#[derive(Default)]
pub struct DownloadQueue {
    notify: Notify,
    /// Téléchargement en cours : id, jeton d'annulation, octets reçus.
    current: Mutex<Option<(i64, CancellationToken, u64)>>,
    /// Octets reçus des dernières secondes (vitesse).
    samples: Mutex<VecDeque<(Instant, u64)>>,
}

impl DownloadQueue {
    /// Un téléchargement de modèle est en cours.
    pub fn busy(&self) -> bool {
        self.current.lock().expect("file").is_some()
    }

    fn progress(&self, id: i64, bytes: u64) {
        if let Some(c) = self.current.lock().expect("file").as_mut()
            && c.0 == id
        {
            c.2 = bytes;
        }
        let mut s = self.samples.lock().expect("file");
        let now = Instant::now();
        s.push_back((now, bytes));
        while s
            .front()
            .is_some_and(|(t, _)| now.duration_since(*t) > Duration::from_secs(5))
        {
            s.pop_front();
        }
    }

    fn speed(&self) -> u64 {
        let s = self.samples.lock().expect("file");
        match (s.front(), s.back()) {
            (Some((t0, b0)), Some((t1, b1))) if t1 > t0 && b1 >= b0 => {
                ((b1 - b0) as f64 / t1.duration_since(*t0).as_secs_f64()) as u64
            }
            _ => 0,
        }
    }

    fn cancel(&self, id: i64) -> bool {
        match self.current.lock().expect("file").as_ref() {
            Some((cur, tok, _)) if *cur == id => {
                tok.cancel();
                true
            }
            _ => false,
        }
    }
}

#[derive(Deserialize)]
pub struct NewDownload {
    repo: String,
    quant: String,
    mmproj: Option<String>,
}

pub async fn create_download(
    State(state): State<AppState>,
    ApiJson(b): ApiJson<NewDownload>,
) -> Result<Json<Value>, ApiError> {
    if !valid_repo(&b.repo) {
        return Err(ApiError::BadRequest);
    }
    let info = repo_info(&state, &b.repo).await?;
    if info.gated {
        return Err(ApiError::GatedRepo);
    }
    let group = info
        .groups
        .iter()
        .find(|g| g.quant == b.quant)
        .ok_or(ApiError::BadRequest)?;
    let mut files: Vec<FileSpec> = group.files.clone();
    if let Some(m) = &b.mmproj {
        files.push(
            info.mmproj
                .iter()
                .find(|f| &f.name == m)
                .ok_or(ApiError::BadRequest)?
                .clone(),
        );
    }
    if files.is_empty() || files.iter().any(|f| !valid_file(&f.name)) {
        return Err(ApiError::BadRequest);
    }
    let main = files[0].name.clone();
    let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM models WHERE repo = ? AND file = ?")
        .bind(&b.repo)
        .bind(&main)
        .fetch_optional(&state.0.db)
        .await?;
    if exists.is_some() {
        return Err(ApiError::AlreadyDownloaded);
    }
    let pending: Vec<String> =
        sqlx::query_scalar("SELECT files FROM downloads WHERE repo = ? AND state IN ('queued','running','paused')")
            .bind(&b.repo)
            .fetch_all(&state.0.db)
            .await?;
    for f in pending {
        let specs: Vec<FileSpec> = serde_json::from_str(&f)?;
        if specs.first().is_some_and(|s| s.name == main) {
            return Err(ApiError::AlreadyQueued);
        }
    }
    let total: u64 = files.iter().map(|f| f.size).sum();
    let dir = &state.config().models_dir;
    let free = download::free_bytes(dir)?;
    if !download::enough_space(free, total) {
        return Err(ApiError::DiskFull);
    }
    let t = now();
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO downloads (repo, files, mmproj, total_bytes, state, created_at, updated_at) \
         VALUES (?, ?, ?, ?, 'queued', ?, ?) RETURNING id",
    )
    .bind(&b.repo)
    .bind(serde_json::to_string(&files)?)
    .bind(&b.mmproj)
    .bind(total as i64)
    .bind(t)
    .bind(t)
    .fetch_one(&state.0.db)
    .await?;
    state.0.downloads.notify.notify_one();
    Ok(Json(json!({ "id": id })))
}

#[derive(sqlx::FromRow)]
struct DownloadRow {
    id: i64,
    repo: String,
    files: String,
    mmproj: Option<String>,
    received_bytes: i64,
    total_bytes: i64,
    state: String,
    error: Option<String>,
}

pub async fn list_downloads(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let rows: Vec<DownloadRow> = sqlx::query_as(
        "SELECT id, repo, files, mmproj, received_bytes, total_bytes, state, error FROM downloads \
         ORDER BY id DESC LIMIT 50",
    )
    .fetch_all(&state.0.db)
    .await?;
    let q = &state.0.downloads;
    let cur = q.current.lock().expect("file").as_ref().map(|(id, _, b)| (*id, *b));
    let speed = q.speed();
    let mut out = Vec::new();
    for r in rows {
        let files: Vec<FileSpec> = serde_json::from_str(&r.files)?;
        let (received, speed) = match cur {
            Some((id, b)) if id == r.id && r.state == "running" => (b, speed),
            _ => (r.received_bytes.max(0) as u64, 0),
        };
        out.push(json!({
            "id": r.id, "repo": r.repo, "files": files, "mmproj": r.mmproj,
            "received_bytes": received, "total_bytes": r.total_bytes, "state": r.state,
            "error": r.error, "speed_bps": speed,
        }));
    }
    Ok(Json(Value::Array(out)))
}

async fn download_state(state: &AppState, id: i64) -> Result<String, ApiError> {
    sqlx::query_scalar("SELECT state FROM downloads WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.0.db)
        .await?
        .ok_or(ApiError::NotFound)
}

async fn set_download_state(state: &AppState, id: i64, st: &str, error: Option<&str>) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE downloads SET state = ?, error = ?, updated_at = ? WHERE id = ?")
        .bind(st)
        .bind(error)
        .bind(now())
        .bind(id)
        .execute(&state.0.db)
        .await
        .map(|_| ())
}

pub async fn pause_download(State(state): State<AppState>, UrlPath(id): UrlPath<i64>) -> Result<Json<Value>, ApiError> {
    let st = download_state(&state, id).await?;
    if !matches!(st.as_str(), "queued" | "running") {
        return Err(ApiError::BadRequest);
    }
    set_download_state(&state, id, "paused", None).await?;
    state.0.downloads.cancel(id);
    Ok(Json(json!({ "ok": true })))
}

pub async fn resume_download(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<i64>,
) -> Result<Json<Value>, ApiError> {
    let st = download_state(&state, id).await?;
    if !matches!(st.as_str(), "paused" | "failed") {
        return Err(ApiError::BadRequest);
    }
    set_download_state(&state, id, "queued", None).await?;
    state.0.downloads.notify.notify_one();
    Ok(Json(json!({ "ok": true })))
}

/// Annule (et supprime les `.part`) ; un téléchargement fini, échoué ou
/// annulé est retiré de la liste.
pub async fn delete_download(
    State(state): State<AppState>,
    UrlPath(id): UrlPath<i64>,
) -> Result<Json<Value>, ApiError> {
    let st = download_state(&state, id).await?;
    if matches!(st.as_str(), "done" | "cancelled" | "failed") {
        if st == "failed" {
            remove_parts(&state, id).await?;
        }
        sqlx::query("DELETE FROM downloads WHERE id = ?")
            .bind(id)
            .execute(&state.0.db)
            .await?;
        return Ok(Json(json!({ "ok": true })));
    }
    set_download_state(&state, id, "cancelled", None).await?;
    // En cours : la boucle supprime les `.part` en voyant l'annulation.
    if !state.0.downloads.cancel(id) {
        remove_parts(&state, id).await?;
    }
    Ok(Json(json!({ "ok": true })))
}

async fn remove_parts(state: &AppState, id: i64) -> Result<(), ApiError> {
    let (repo, files): (String, String) = sqlx::query_as("SELECT repo, files FROM downloads WHERE id = ?")
        .bind(id)
        .fetch_one(&state.0.db)
        .await?;
    let specs: Vec<FileSpec> = serde_json::from_str(&files)?;
    for f in specs {
        if let Ok(p) = model_path(&state.config().models_dir, &repo, &f.name) {
            let _ = tokio::fs::remove_file(part_of(&p)).await;
        }
    }
    Ok(())
}

/// Au lancement : `running` → `queued`, `.part` orphelins supprimés, boucle.
pub async fn boot(state: &AppState) -> anyhow::Result<()> {
    sqlx::query("UPDATE downloads SET state = 'queued' WHERE state = 'running'")
        .execute(&state.0.db)
        .await?;
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT repo, files FROM downloads WHERE state IN ('queued','paused','failed')")
            .fetch_all(&state.0.db)
            .await?;
    let mut keep = HashSet::new();
    for (repo, files) in rows {
        for f in serde_json::from_str::<Vec<FileSpec>>(&files)? {
            if let Ok(p) = model_path(&state.config().models_dir, &repo, &f.name) {
                keep.insert(part_of(&p));
            }
        }
    }
    let dir = state.config().models_dir.clone();
    tokio::task::spawn_blocking(move || remove_orphans(&dir, &keep, 0)).await?;
    let r = state.clone();
    tokio::spawn(async move { refresh_meta(&r).await });
    let s = state.clone();
    tokio::spawn(async move { download_loop(s).await });
    Ok(())
}

/// Relit l'en-tête des modèles locaux : un modèle téléchargé avant l'ajout
/// d'un champ (experts MoE, couches MTP…) le reçoit sans retéléchargement.
async fn refresh_meta(state: &AppState) {
    let rows: Vec<(i64, String, String, String)> = sqlx::query_as("SELECT id, repo, shards, meta FROM models")
        .fetch_all(&state.0.db)
        .await
        .unwrap_or_default();
    for (id, repo, shards, old) in rows {
        let names: Vec<String> = serde_json::from_str(&shards).unwrap_or_default();
        let Ok(paths) = names
            .iter()
            .map(|n| model_path(&state.config().models_dir, &repo, n))
            .collect::<Result<Vec<_>, _>>()
        else {
            continue;
        };
        let read = tokio::task::spawn_blocking(move || gguf::read_shards(&paths)).await;
        let Ok(Ok(meta)) = read else { continue };
        let Ok(new) = serde_json::to_string(&meta) else {
            continue;
        };
        let same = serde_json::from_str::<Value>(&old).ok() == serde_json::from_str::<Value>(&new).ok();
        if !same {
            let _ = sqlx::query("UPDATE models SET meta = ? WHERE id = ?")
                .bind(&new)
                .bind(id)
                .execute(&state.0.db)
                .await;
        }
    }
}

fn remove_orphans(dir: &Path, keep: &HashSet<PathBuf>, depth: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() && depth < 4 {
            remove_orphans(&p, keep, depth + 1);
        } else if ft.is_file() && p.extension().is_some_and(|x| x == "part") && !keep.contains(&p) {
            tracing::info!("fichier partiel orphelin supprimé : {}", p.display());
            let _ = std::fs::remove_file(&p);
        }
    }
}

async fn download_loop(state: AppState) {
    loop {
        let next: Result<Option<DownloadRow>, _> = sqlx::query_as(
            "SELECT id, repo, files, mmproj, received_bytes, total_bytes, state, error FROM downloads \
             WHERE state = 'queued' ORDER BY id LIMIT 1",
        )
        .fetch_optional(&state.0.db)
        .await;
        match next {
            Ok(Some(row)) => run_download(&state, row).await,
            Ok(None) => {
                let _ = tokio::time::timeout(Duration::from_secs(30), state.0.downloads.notify.notified()).await;
            }
            Err(e) => {
                tracing::error!("file des téléchargements : {e}");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

fn error_code(e: &DownloadError) -> String {
    match e {
        DownloadError::Cancelled => "cancelled".into(),
        DownloadError::Sha256Mismatch => "sha256_mismatch".into(),
        DownloadError::Status(s) => format!("http_{s}"),
        DownloadError::Network(_) => "network".into(),
        DownloadError::Io(m) if m == "disk_full" => "disk_full".into(),
        DownloadError::Io(_) => "io".into(),
    }
}

async fn run_download(state: &AppState, row: DownloadRow) {
    let id = row.id;
    let token = CancellationToken::new();
    *state.0.downloads.current.lock().expect("file") = Some((id, token.clone(), 0));
    state.0.downloads.samples.lock().expect("file").clear();
    let _ = set_download_state(state, id, "running", None).await;
    let res = download_files(state, &row, &token).await;
    let got = state
        .0
        .downloads
        .current
        .lock()
        .expect("file")
        .take()
        .map(|(_, _, b)| b);
    // Octets reçus gardés pour l'affichage (pause, échec).
    if let Some(b) = got.filter(|b| *b > 0) {
        let _ = sqlx::query("UPDATE downloads SET received_bytes = ? WHERE id = ?")
            .bind(b as i64)
            .bind(id)
            .execute(&state.0.db)
            .await;
    }
    match res {
        Ok(()) => {
            if let Err(e) = finish(state, &row).await {
                tracing::error!("téléchargement {id} : enregistrement du modèle : {e:#}");
                let _ = set_download_state(state, id, "failed", Some("register_failed")).await;
            } else {
                let _ = set_download_state(state, id, "done", None).await;
                crate::feed::emit(
                    state,
                    crate::feed::Note::new("download", "success", format!("Téléchargement fini : {}", row.repo))
                        .link("#models"),
                );
            }
        }
        Err(DownloadError::Cancelled) => {
            // Pause ou annulation : la route a déjà écrit l'état voulu.
            if download_state(state, id).await.ok().as_deref() == Some("cancelled") {
                let _ = remove_parts(state, id).await;
            }
        }
        Err(e) => {
            tracing::warn!("téléchargement {id} : {e}");
            let code = error_code(&e);
            if e == DownloadError::Sha256Mismatch {
                let _ = remove_parts(state, id).await;
            }
            let _ = set_download_state(state, id, "failed", Some(&code)).await;
            crate::feed::emit(
                state,
                crate::feed::Note::new("download", "error", format!("Téléchargement en échec : {}", row.repo))
                    .body(e.to_string())
                    .link("#models"),
            );
        }
    }
}

async fn download_files(state: &AppState, row: &DownloadRow, token: &CancellationToken) -> Result<(), DownloadError> {
    let files: Vec<FileSpec> = serde_json::from_str(&row.files).map_err(|e| DownloadError::Io(e.to_string()))?;
    let dir = &state.config().models_dir;
    let mut done: u64 = 0;
    for f in &files {
        let dest =
            model_path(dir, &row.repo, &f.name).map_err(|_| DownloadError::Io("nom de fichier refusé".into()))?;
        if tokio::fs::metadata(&dest).await.is_ok_and(|m| m.len() == f.size) {
            done += f.size;
            continue;
        }
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let part = part_of(&dest);
        let have = tokio::fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0);
        let free = download::free_bytes(dir)?;
        if !download::enough_space(free, f.size.saturating_sub(have)) {
            return Err(DownloadError::Io("disk_full".into()));
        }
        let q = &state.0.downloads;
        let base = done;
        let url = state.0.hf.file_url(&row.repo, &f.name);
        download::fetch(&state.0.http, &url, &part, f, token, &|n| q.progress(row.id, base + n)).await?;
        tokio::fs::rename(&part, &dest).await?;
        done += f.size;
        let _ = sqlx::query("UPDATE downloads SET received_bytes = ?, updated_at = ? WHERE id = ?")
            .bind(done as i64)
            .bind(now())
            .bind(row.id)
            .execute(&state.0.db)
            .await;
    }
    Ok(())
}

/// Fin d'un téléchargement : ligne `models`, GGUF relu, preset auto.
async fn finish(state: &AppState, row: &DownloadRow) -> anyhow::Result<()> {
    let files: Vec<FileSpec> = serde_json::from_str(&row.files)?;
    let shards: Vec<&FileSpec> = files
        .iter()
        .filter(|f| row.mmproj.as_deref() != Some(f.name.as_str()))
        .collect();
    let main = shards
        .first()
        .ok_or_else(|| anyhow::anyhow!("aucun fichier de poids"))?;
    let paths = shards
        .iter()
        .map(|f| model_path(&state.config().models_dir, &row.repo, &f.name))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let meta = tokio::task::spawn_blocking(move || gguf::read_shards(&paths)).await??;
    let sha: HashMap<&str, &str> = files.iter().map(|f| (f.name.as_str(), f.sha256.as_str())).collect();
    let names: Vec<&str> = shards.iter().map(|f| f.name.as_str()).collect();
    let size: u64 = shards.iter().map(|f| f.size).sum();
    let model_id: i64 = sqlx::query_scalar(
        "INSERT INTO models (repo, file, shards, mmproj, size_bytes, sha256, meta, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(&row.repo)
    .bind(&main.name)
    .bind(serde_json::to_string(&names)?)
    .bind(&row.mmproj)
    .bind(size as i64)
    .bind(serde_json::to_string(&sha)?)
    .bind(serde_json::to_string(&meta)?)
    .bind(now())
    .fetch_one(&state.0.db)
    .await?;
    crate::presets::create_auto(state, model_id).await?;
    Ok(())
}

// ---- Modèles locaux ----

#[derive(sqlx::FromRow)]
pub struct ModelRow {
    pub id: i64,
    pub repo: String,
    pub file: String,
    pub shards: String,
    pub mmproj: Option<String>,
    pub size_bytes: i64,
    pub meta: String,
}

pub async fn model(state: &AppState, id: i64) -> Result<ModelRow, ApiError> {
    sqlx::query_as("SELECT id, repo, file, shards, mmproj, size_bytes, meta FROM models WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.0.db)
        .await?
        .ok_or(ApiError::NotFound)
}

pub async fn list_models(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let rows: Vec<ModelRow> =
        sqlx::query_as("SELECT id, repo, file, shards, mmproj, size_bytes, meta FROM models ORDER BY repo, file")
            .fetch_all(&state.0.db)
            .await?;
    let presets: Vec<(i64, i64)> = sqlx::query_as("SELECT id, model_id FROM presets ORDER BY id")
        .fetch_all(&state.0.db)
        .await?;
    let mut out = Vec::new();
    for r in rows {
        let ids: Vec<i64> = presets.iter().filter(|(_, m)| *m == r.id).map(|(p, _)| *p).collect();
        out.push(json!({
            "id": r.id, "repo": r.repo, "file": r.file,
            "shards": serde_json::from_str::<Value>(&r.shards)?,
            "mmproj": r.mmproj, "size_bytes": r.size_bytes,
            "meta": serde_json::from_str::<Value>(&r.meta)?, "presets": ids,
        }));
    }
    Ok(Json(Value::Array(out)))
}

pub async fn delete_model(State(state): State<AppState>, UrlPath(id): UrlPath<i64>) -> Result<Json<Value>, ApiError> {
    let m = model(&state, id).await?;
    if let Some(active) = setting(&state, "preset.active")
        .await?
        .and_then(|v| v.parse::<i64>().ok())
    {
        let used: Option<i64> = sqlx::query_scalar("SELECT model_id FROM presets WHERE id = ?")
            .bind(active)
            .fetch_optional(&state.0.db)
            .await?;
        if used == Some(id) {
            return Err(ApiError::ModelInUse);
        }
    }
    remove_model(&state, &m).await?;
    Ok(Json(json!({ "ok": true })))
}

/// Supprime les fichiers d'un modèle et sa ligne (ses presets partent avec,
/// `ON DELETE CASCADE`).
pub async fn remove_model(state: &AppState, m: &ModelRow) -> Result<(), ApiError> {
    let dir = &state.config().models_dir;
    let mut names: Vec<String> = serde_json::from_str(&m.shards)?;
    names.extend(m.mmproj.clone());
    for n in &names {
        let p = model_path(dir, &m.repo, n)?;
        let _ = tokio::fs::remove_file(&p).await;
        // Dossiers vides jusqu'à `models_dir`.
        let mut d = p.parent().map(Path::to_path_buf);
        while let Some(x) = d {
            if x == *dir || tokio::fs::remove_dir(&x).await.is_err() {
                break;
            }
            d = x.parent().map(Path::to_path_buf);
        }
    }
    sqlx::query("DELETE FROM models WHERE id = ?")
        .bind(m.id)
        .execute(&state.0.db)
        .await?;
    Ok(())
}

/// Verdict de place d'un modèle local avec les réglages d'un preset.
pub async fn local_fit(
    state: &AppState,
    m: &ModelRow,
    s: &enginex_engine::args::PresetSettings,
) -> Result<FitReport, ApiError> {
    let meta: GgufMeta = serde_json::from_str(&m.meta)?;
    let mmproj_bytes = match (&m.mmproj, s.mmproj) {
        (Some(f), true) => {
            let p = model_path(&state.config().models_dir, &m.repo, f)?;
            tokio::fs::metadata(p).await.map(|x| x.len()).unwrap_or(0)
        }
        _ => 0,
    };
    let draft_bytes = match (s.spec.draft, s.spec.draft_model_id) {
        (enginex_engine::args::DraftKind::Model, Some(d)) if d != m.id => {
            model(state, d).await.map(|d| d.size_bytes.max(0) as u64).unwrap_or(0)
        }
        _ => 0,
    };
    let (cards, _) = cards(state).await;
    Ok(fit::estimate(
        &FitInput {
            meta: &meta,
            weights_bytes: m.size_bytes.max(0) as u64,
            mmproj_bytes,
            ctx: s.ctx,
            parallel: s.parallel,
            kv: KvType::parse(&s.kv_type).unwrap_or(KvType::Q8_0),
            fit_target_mib: s.fit_target_mib,
            draft_bytes,
            flash_attn: s.flash_attn,
            ubatch: s.ubatch.unwrap_or(0),
            mtp: s.spec.draft == enginex_engine::args::DraftKind::Mtp && meta.has_mtp(),
        },
        &cards,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation() {
        assert!(valid_repo("unsloth/Qwen3-0.6B-GGUF"));
        assert!(!valid_repo("../etc"));
        assert!(!valid_repo("a/b/c"));
        assert!(!valid_repo("a/.."));
        assert!(valid_file("BF16/M-BF16-00001-of-00002.gguf"));
        assert!(!valid_file("../x.gguf"));
        assert!(!valid_file("/x.gguf"));
        assert!(!valid_file("x.bin"));
        let d = Path::new("/models");
        assert_eq!(model_path(d, "a/b", "c.gguf").unwrap(), Path::new("/models/a/b/c.gguf"));
        assert!(model_path(d, "a/b", "x/../../c.gguf").is_err());
    }
}
