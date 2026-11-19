//! Vérification après un tour : recette (`.enginex/verify.toml` ou détectée),
//! étapes lancées dans la sandbox par `command/exec` (mode tamponné), badge
//! par tour, tours de correction automatiques bornés par `verify.max_fixes`.

use std::{path::Path, sync::Arc, time::Duration};

use axum::{
    Json,
    extract::{Path as UrlPath, State},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    bridge::{self, Hub},
    error::ApiError,
    files, sessions, snapshots,
    state::{AppState, now},
    system::setting,
};

pub const DEVELOPER_INSTRUCTIONS: &str = "Après chaque tour qui modifie des fichiers, EngineX lance la \
vérification décrite dans `.enginex/verify.toml` (ou une recette détectée) et te renvoie les échecs. Écris \
ou adapte les tests du code que tu modifies et lance-les avant de dire que c'est fini. Tu peux créer ou \
modifier `.enginex/verify.toml` (format : [[step]] name, run, timeout). Chaque réponse a une limite de \
sortie : écris un fichier de plus de 300 lignes en plusieurs fois (un squelette court, puis une section par \
commande avec apply_patch ou `cat >>`), jamais d'un seul bloc.";

pub const RECIPE: &str = ".enginex/verify.toml";
const RECIPE_MAX: usize = 64 * 1024;
const MAX_STEPS: usize = 20;
const OUTPUT_LINES: usize = 200;
const OUTPUT_BYTES: usize = 16 * 1024;
const OUTPUT_CAP: u64 = 1024 * 1024;

fn default_timeout() -> u64 {
    600
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Step {
    pub name: String,
    pub run: String,
    #[serde(default = "default_timeout")]
    pub timeout: u64,
}

#[derive(Deserialize)]
struct RecipeFile {
    #[serde(default)]
    step: Vec<Step>,
}

#[derive(Debug, PartialEq)]
pub enum Recipe {
    File(Vec<Step>),
    Detected(Vec<Step>),
    None,
    Invalid(String),
}

/// Lit et valide un `verify.toml`.
pub fn parse(text: &str) -> Result<Vec<Step>, String> {
    let f: RecipeFile = toml::from_str(text).map_err(|e| e.to_string())?;
    if f.step.is_empty() {
        return Err("aucune étape [[step]]".into());
    }
    if f.step.len() > MAX_STEPS {
        return Err(format!("{MAX_STEPS} étapes au plus"));
    }
    for s in &f.step {
        if s.name.trim().is_empty() || s.run.trim().is_empty() {
            return Err("chaque étape a un `name` et un `run`".into());
        }
        if !(10..=3600).contains(&s.timeout) {
            return Err(format!("étape « {} » : timeout entre 10 et 3600 s", s.name));
        }
    }
    Ok(f.step)
}

fn step(name: &str, run: &str) -> Step {
    Step {
        name: name.into(),
        run: run.into(),
        timeout: default_timeout(),
    }
}

/// Recette détectée à la racine du dossier de travail.
pub fn detect(work: &Path) -> Vec<Step> {
    let has = |p: &str| files::kind_beneath(work, p).is_some();
    let mut out = Vec::new();
    if let Some(pkg) =
        files::read_beneath(work, "package.json", RECIPE_MAX).and_then(|b| serde_json::from_slice::<Value>(&b).ok())
    {
        let (pm, install) = if has("pnpm-lock.yaml") {
            ("corepack pnpm", "corepack pnpm install --frozen-lockfile")
        } else if has("yarn.lock") {
            ("corepack yarn", "corepack yarn install --frozen-lockfile")
        } else if has("package-lock.json") {
            ("npm", "npm ci")
        } else {
            ("npm", "npm install")
        };
        if !has("node_modules") {
            out.push(step("installation", install));
        }
        for script in ["lint", "build", "test"] {
            let Some(cmd) = pkg["scripts"][script].as_str() else {
                continue;
            };
            if script == "test" && cmd.contains("no test specified") {
                continue;
            }
            out.push(step(script, &format!("{pm} run {script}")));
        }
    }
    if has("Cargo.toml") {
        out.push(step("cargo build", "cargo build --all-targets"));
        out.push(step("cargo test", "cargo test"));
    }
    if has("go.mod") {
        out.push(step("go build", "go build ./..."));
        out.push(step("go test", "go test ./..."));
    }
    if files::read_beneath(work, "pyproject.toml", RECIPE_MAX)
        .is_some_and(|b| String::from_utf8_lossy(&b).contains("pytest"))
    {
        out.push(step("pytest", "uv run pytest -q"));
    }
    out
}

pub fn load(work: &Path) -> Recipe {
    if let Some(b) = files::read_beneath(work, RECIPE, RECIPE_MAX + 1) {
        if b.len() > RECIPE_MAX {
            return Recipe::Invalid("verify.toml : 64 Kio au plus".into());
        }
        return match parse(&String::from_utf8_lossy(&b)) {
            Ok(s) => Recipe::File(s),
            Err(e) => Recipe::Invalid(format!("verify.toml : {e}")),
        };
    }
    let d = detect(work);
    if d.is_empty() {
        Recipe::None
    } else {
        Recipe::Detected(d)
    }
}

pub async fn enabled(state: &AppState) -> bool {
    setting(state, "verify.enabled").await.ok().flatten().as_deref() != Some("0")
}

pub async fn max_fixes(state: &AppState) -> i64 {
    crate::sandboxes::setting_num(state, "verify.max_fixes", 3, 0, 10).await
}

/// Fin de `s` : `lines` dernières lignes, `bytes` derniers octets au plus.
fn tail_output(s: &str) -> String {
    let t = enginex_sandbox::tail(s, OUTPUT_LINES);
    if t.len() <= OUTPUT_BYTES {
        return t;
    }
    let mut cut = t.len() - OUTPUT_BYTES;
    while !t.is_char_boundary(cut) {
        cut += 1;
    }
    t[cut..].to_string()
}

/// Fin d'un tour en tâche de fond : instantané d'après, puis vérification.
/// Fonction simple (future en boîte) : coupe le cycle de types pont →
/// vérification → tour de correction → pont.
pub fn spawn_after_turn(
    state: AppState,
    hub: Arc<Hub>,
    tid: String,
    status: String,
    origin: &'static str,
    pre: Option<String>,
) {
    let fut: std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> = Box::pin(async move {
        let changes = match &pre {
            Some(p) => hub.post_snapshot(&state, &tid, p).await,
            None => None,
        };
        after_turn(&state, &hub, &tid, &status, origin, changes.as_ref()).await;
    });
    tokio::spawn(fut);
}

/// Après `turn/completed` : vérification si activée, s'il y a une recette,
/// et si le tour a changé des fichiers ou est un tour de correction.
pub async fn after_turn(
    state: &AppState,
    hub: &Arc<Hub>,
    tid: &str,
    status: &str,
    origin: &str,
    changes: Option<&Value>,
) {
    // Pas de vérification après la mise à jour de la mémoire.
    if origin == "memory" {
        return;
    }
    if status != "completed" || !enabled(state).await {
        return;
    }
    let changed = changes
        .and_then(|c| c["files"].as_array())
        .is_some_and(|f| !f.is_empty());
    if !changed && origin != "verify" {
        return;
    }
    run(state, hub, tid).await;
}

/// Une vérification du tour `tid` (puis, sur échec, un tour de correction
/// si le compte le permet).
pub async fn run(state: &AppState, hub: &Arc<Hub>, tid: &str) {
    let Ok(row) = sessions::row(state, &hub.id).await else {
        return;
    };
    let Ok(folder) = snapshots::folder(state, &row).await else {
        return;
    };
    let recipe = {
        let w = folder.work.clone();
        tokio::task::spawn_blocking(move || load(&w))
            .await
            .unwrap_or(Recipe::None)
    };
    let steps = match recipe {
        Recipe::None => {
            hub.emit_event("verify", json!({"turn_id": tid, "status": "none"}));
            return;
        }
        Recipe::Invalid(e) => {
            if hub.verify_begin(tid) {
                let fixes = hub.fixes_done();
                let id = insert(state, &hub.id, tid, fixes + 1, "error", &json!([{"error": e}])).await;
                emit(state, hub, tid, id, "error", fixes + 1, &json!([{"error": e}])).await;
                hub.verify_end();
            }
            return;
        }
        Recipe::File(s) | Recipe::Detected(s) => s,
    };
    // Un retour arrière en cours sur ce dossier finit d'abord.
    drop(snapshots::lock(state, &folder).await);
    if !hub.verify_begin(tid) {
        return;
    }
    let attempt = hub.fixes_done() + 1;
    let id = insert(state, &hub.id, tid, attempt, "running", &json!([])).await;
    emit(state, hub, tid, id, "running", attempt, &json!([])).await;
    let (status, results) = match hub.ready(state).await {
        Err(e) => ("error", vec![json!({"error": e})]),
        Ok((client, cwd)) => run_steps(state, hub, &client, &cwd, &steps, tid, id, attempt).await,
    };
    let results = Value::Array(results);
    finish(state, id, status, &results).await;
    hub.verify_end();
    hub.record(state);
    emit(state, hub, tid, id, status, attempt, &results).await;
    if status == "passed"
        && let Ok(Some(post)) =
            sqlx::query_scalar::<_, Option<String>>("SELECT snap_post FROM chat_turns WHERE session_id = ? AND id = ?")
                .bind(&hub.id)
                .bind(tid)
                .fetch_optional(state.db())
                .await
                .map(Option::flatten)
        && let Err(e) = snapshots::set_ref(state, &folder, &snapshots::green_ref(&hub.id), &post).await
    {
        tracing::warn!("ref green : {e:#}");
    }
    if status == "failed" && i64::from(hub.fixes_done()) < max_fixes(state).await {
        let fixes = hub.fixes_done() + 1;
        let max = max_fixes(state).await;
        let text = fix_message(&results, attempt, fixes, max);
        let user = json!({
            "type": "userMessage",
            "id": format!("ex-{}", uuid::Uuid::new_v4()),
            "text": state.masker().mask(&text),
            "images": [],
            "origin": "verify",
        });
        if hub.begin(user, "verify").is_ok() {
            hub.add_fix();
            tokio::spawn(hub.clone().run_turn(state.clone(), text, Vec::new()));
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_steps(
    state: &AppState,
    hub: &Arc<Hub>,
    client: &enginex_codex::Client,
    cwd: &str,
    steps: &[Step],
    tid: &str,
    id: i64,
    attempt: u32,
) -> (&'static str, Vec<Value>) {
    let mut results = Vec::new();
    for s in steps {
        let pid = format!("verify-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        if !hub.verify_step(Some(pid.clone())) {
            return ("cancelled", results);
        }
        let mut params =
            enginex_codex::types::command_exec(&["bash", "-lc", &s.run], cwd, Some(&pid), None, Some(s.timeout * 1000));
        params["outputBytesCap"] = json!(OUTPUT_CAP);
        let started = std::time::Instant::now();
        let r = client
            .request_with_timeout("command/exec", params, Some(Duration::from_secs(s.timeout + 30)))
            .await;
        let ms = started.elapsed().as_millis() as u64;
        if hub.verify_cancelled() {
            return ("cancelled", results);
        }
        let (code, output, timed_out) = match r {
            Ok(v) => {
                let out = format!(
                    "{}{}",
                    v["stdout"].as_str().unwrap_or(""),
                    v["stderr"].as_str().unwrap_or("")
                );
                let code = v["exitCode"].as_i64().unwrap_or(-1);
                (code, out, ms >= s.timeout * 1000)
            }
            Err(enginex_codex::Error::Timeout(_)) => (-1, String::new(), true),
            Err(e) => {
                results.push(json!({"name": s.name, "run": s.run, "error": e.to_string()}));
                return ("error", results);
            }
        };
        let output = state.masker().mask(&tail_output(&output)).into_owned();
        results.push(
            json!({"name": s.name, "run": s.run, "exit_code": code, "duration_ms": ms,
            "timed_out": timed_out, "output": output}),
        );
        let _ = sqlx::query("UPDATE verifications SET steps = ? WHERE id = ?")
            .bind(Value::Array(results.clone()).to_string())
            .bind(id)
            .execute(state.db())
            .await;
        emit(state, hub, tid, id, "running", attempt, &Value::Array(results.clone())).await;
        if code != 0 || timed_out {
            return ("failed", results);
        }
    }
    ("passed", results)
}

/// Message du tour de correction (fin de la sortie en bloc de code).
pub fn fix_message(results: &Value, attempt: u32, fix: u32, max: i64) -> String {
    let last = results
        .as_array()
        .and_then(|a| a.last())
        .cloned()
        .unwrap_or(Value::Null);
    let delay = if last["timed_out"] == true {
        " [délai dépassé]"
    } else {
        ""
    };
    format!(
        "La vérification automatique d'EngineX a échoué (essai {attempt}, correction {fix}/{max}).\n\
         Étape « {} » : `{}` → code {}{delay}.\nFin de la sortie :\n```\n{}\n```\n\
         Corrige le problème puis termine ton tour. EngineX relancera la vérification.",
        last["name"].as_str().unwrap_or(""),
        last["run"].as_str().unwrap_or(""),
        last["exit_code"].as_i64().unwrap_or(-1),
        last["output"].as_str().unwrap_or("").replace("```", "'''"),
    )
}

async fn insert(state: &AppState, sid: &str, tid: &str, attempt: u32, status: &str, steps: &Value) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO verifications (session_id, turn_id, attempt, status, steps, started_at, completed_at) \
         VALUES (?, ?, ?, ?, ?, ?, CASE WHEN ? = 'running' THEN NULL ELSE ? END) RETURNING id",
    )
    .bind(sid)
    .bind(tid)
    .bind(attempt)
    .bind(status)
    .bind(steps.to_string())
    .bind(now())
    .bind(status)
    .bind(now())
    .fetch_one(state.db())
    .await
    .unwrap_or(0)
}

async fn finish(state: &AppState, id: i64, status: &str, steps: &Value) {
    let _ = sqlx::query("UPDATE verifications SET status = ?, steps = ?, completed_at = ? WHERE id = ?")
        .bind(status)
        .bind(steps.to_string())
        .bind(now())
        .bind(id)
        .execute(state.db())
        .await;
}

async fn emit(state: &AppState, hub: &Hub, tid: &str, id: i64, status: &str, attempt: u32, steps: &Value) {
    hub.emit_event(
        "verify",
        json!({"turn_id": tid, "id": id, "status": status, "attempt": attempt,
            "fixes_done": hub.fixes_done(), "max_fixes": max_fixes(state).await, "steps": steps}),
    );
}

/// Vérifications d'une session, par tour (détail de la session).
/// `(id, turn_id, attempt, status, steps, started_at, finished_at)`.
type VerifyRow = (i64, String, i64, String, String, i64, Option<i64>);

pub async fn by_turn(state: &AppState, sid: &str) -> Result<std::collections::HashMap<String, Vec<Value>>, ApiError> {
    let rows: Vec<VerifyRow> = sqlx::query_as(
        "SELECT id, turn_id, attempt, status, steps, started_at, completed_at FROM verifications \
         WHERE session_id = ? ORDER BY id",
    )
    .bind(sid)
    .fetch_all(state.db())
    .await?;
    let mut out: std::collections::HashMap<String, Vec<Value>> = Default::default();
    for (id, tid, attempt, status, steps, s, c) in rows {
        out.entry(tid.clone()).or_default().push(json!({
            "id": id, "turn_id": tid, "attempt": attempt, "status": status,
            "steps": serde_json::from_str::<Value>(&steps).unwrap_or(Value::Null),
            "started_at": s, "completed_at": c,
        }));
    }
    Ok(out)
}

// ---- Routes ----

async fn work_of(state: &AppState, id: &str) -> Result<std::path::PathBuf, ApiError> {
    let r = sessions::row(state, id).await?;
    Ok(snapshots::folder(state, &r).await?.work)
}

pub async fn get(State(state): State<AppState>, UrlPath(id): UrlPath<String>) -> Result<Json<Value>, ApiError> {
    let w = work_of(&state, &id).await?;
    let r = tokio::task::spawn_blocking(move || load(&w))
        .await
        .map_err(|e| ApiError::Internal(e.into()))?;
    Ok(Json(match r {
        Recipe::File(s) => json!({"source": "file", "steps": s}),
        Recipe::Detected(s) => json!({"source": "detected", "steps": s}),
        Recipe::None => json!({"source": "none", "steps": []}),
        Recipe::Invalid(e) => json!({"source": "file", "steps": [], "error": e}),
    }))
}

/// Écrit la recette détectée dans `.enginex/verify.toml`.
pub async fn save(State(state): State<AppState>, UrlPath(id): UrlPath<String>) -> Result<Json<Value>, ApiError> {
    let w = work_of(&state, &id).await?;
    tokio::task::spawn_blocking(move || -> Result<(), ApiError> {
        if files::kind_beneath(&w, RECIPE).is_some() {
            return Err(ApiError::RecipeExists);
        }
        let steps = detect(&w);
        if steps.is_empty() {
            return Err(ApiError::NoRecipe);
        }
        let mut text = String::from("# Vérification lancée par EngineX après chaque tour qui change des fichiers.\n");
        for s in steps {
            text.push_str(&format!(
                "\n[[step]]\nname = {}\nrun = {}\ntimeout = {}\n",
                toml_str(&s.name),
                toml_str(&s.run),
                s.timeout
            ));
        }
        files::create_beneath(&w, RECIPE, text.as_bytes()).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                ApiError::RecipeExists
            } else {
                ApiError::Internal(e.into())
            }
        })
    })
    .await
    .map_err(|e| ApiError::Internal(e.into()))??;
    Ok(Json(json!({ "ok": true })))
}

fn toml_str(s: &str) -> String {
    Value::String(s.to_string()).to_string()
}

/// Vérification à la main (sandbox démarrée au besoin), sur le dernier tour.
pub async fn run_now(State(state): State<AppState>, UrlPath(id): UrlPath<String>) -> Result<Json<Value>, ApiError> {
    sessions::row(&state, &id).await?;
    let hub = bridge::hub(&state, &id);
    if hub.busy() {
        return Err(ApiError::TurnInProgress);
    }
    let w = work_of(&state, &id).await?;
    if matches!(
        tokio::task::spawn_blocking(move || load(&w))
            .await
            .unwrap_or(Recipe::None),
        Recipe::None
    ) {
        return Err(ApiError::NoRecipe);
    }
    let tid: Option<String> = sqlx::query_scalar(
        "SELECT id FROM chat_turns WHERE session_id = ? ORDER BY started_at DESC, rowid DESC LIMIT 1",
    )
    .bind(&id)
    .fetch_optional(state.db())
    .await?;
    let tid = tid.ok_or(ApiError::NotFound)?;
    state.0.sandboxes.backend()?;
    tokio::spawn(async move { run(&state, &hub, &tid).await });
    Ok(Json(json!({ "ok": true })))
}
