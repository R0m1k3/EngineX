//! Copie séparée : une session travaille dans un clone du dépôt git du topic,
//! sur sa propre branche, puis « Ramener dans le topic » fusionne la branche.
//! Tout git passe par le conteneur d'aide (sans réseau) : un `.git/config` ou
//! un hook écrit par la sandbox n'exécute rien avec les droits d'EngineX.

use std::time::Duration;

use axum::{
    Json,
    extract::{Path as UrlPath, State},
};
use enginex_sandbox::HelperSpec;
use serde_json::{Value, json};

use crate::{error::ApiError, sandboxes, sessions, snapshots, state::AppState, workspace::TopicInfo};

/// Création d'une copie (clone d'un gros dépôt compris).
pub const CREATE_TIMEOUT: Duration = Duration::from_secs(600);
const BRING_TIMEOUT: Duration = Duration::from_secs(300);

/// Réglages communs : aucun hook, aucune surveillance, propriétaire du dépôt
/// non contrôlé (la sandbox écrit en uid 1000).
macro_rules! git_fn {
    () => {
        r#"set -eu
export GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null HOME=/tmp GIT_TERMINAL_PROMPT=0
g() { git -c safe.directory='*' -c core.hooksPath=/dev/null -c core.fsmonitor=false \
  -c commit.gpgsign=false -c protocol.file.allow=always -c submodule.recurse=false "$@"; }
"#
    };
}

const CREATE: &str = concat!(
    git_fn!(),
    r#"g clone -q --no-hardlinks --template= "$TOPIC" "$COPY"
cd "$COPY"
g switch -q -c "$BRANCH"
if url=$(g -C "$TOPIC" remote get-url origin 2>/dev/null); then
  g remote set-url origin "$url"
else
  g remote remove origin
fi
"#
);

const BRING: &str = concat!(
    git_fn!(),
    r#"cd "$COPY"
if [ -n "$(g status --porcelain)" ]; then
  g add -A
  g commit -q -m "$MSG"
fi
cd "$TOPIC"
g fetch -q "$COPY" "+refs/heads/$BRANCH:refs/heads/$BRANCH"
if [ -n "$(g status --porcelain --untracked-files=no)" ]; then exit 10; fi
if ! g merge -q --no-edit "$BRANCH" > /dev/null 2>&1; then
  g merge --abort > /dev/null 2>&1 || true
  exit 11
fi
"#
);

/// Branche d'une copie : `enginex/<slug du topic>-<8 car. de l'id>`.
pub fn branch_name(topic_slug: &str, session_id: &str) -> String {
    let slug: String = topic_slug
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let short: String = session_id.chars().filter(|c| *c != '-').take(8).collect();
    format!("enginex/{}-{short}", slug.trim_matches('-'))
}

pub fn is_git(topic: &TopicInfo) -> bool {
    crate::files::kind_beneath(&topic.dir, ".git").is_some()
}

async fn helper(
    state: &AppState,
    session_id: &str,
    binds: Vec<(std::path::PathBuf, &'static str, bool)>,
    script: &'static str,
    env: Vec<(String, String)>,
    timeout: Duration,
) -> Result<enginex_sandbox::HelperOutcome, ApiError> {
    let backend = state.0.sandboxes.backend()?;
    let spec = HelperSpec {
        session_id: session_id.to_string(),
        owner: state.0.sandboxes.instance().to_string(),
        binds,
        script,
        env,
        network: false,
    };
    backend
        .run_helper(&spec, timeout)
        .await
        .map_err(|e| ApiError::CopyFailed(state.masker().mask(&format!("{e:#}")).into_owned()))
}

/// Crée la copie de la session `session_id` (dossier vide créé ici) ; rend la
/// branche. Échec : dossier supprimé, `copy_failed` avec la fin du journal.
pub async fn create(state: &AppState, topic: &TopicInfo, session_id: &str) -> Result<String, ApiError> {
    if !is_git(topic) {
        return Err(ApiError::NotGit);
    }
    let branch = branch_name(&topic.slug, session_id);
    let dir = snapshots::copy_dir(state, session_id);
    std::fs::create_dir_all(&dir).map_err(|e| ApiError::Internal(e.into()))?;
    let mut env = sandboxes::git_env(state).await;
    env.push(("BRANCH".into(), branch.clone()));
    let r = helper(
        state,
        session_id,
        vec![(topic.dir.clone(), "TOPIC", false), (dir.clone(), "COPY", true)],
        CREATE,
        env,
        CREATE_TIMEOUT,
    )
    .await;
    match r {
        Ok(o) if o.code == 0 => Ok(branch),
        other => {
            let _ = std::fs::remove_dir_all(sandboxes::session_dir(state, session_id));
            Err(match other {
                Ok(o) => ApiError::CopyFailed(state.masker().mask(o.stderr.trim()).into_owned()),
                Err(e) => e,
            })
        }
    }
}

/// « Ramener dans le topic » : commit de ce qui reste, branche placée dans le
/// dépôt du topic, fusion si le topic est propre et sans conflit.
pub async fn bring_back(State(state): State<AppState>, UrlPath(id): UrlPath<String>) -> Result<Json<Value>, ApiError> {
    let r = sessions::row(&state, &id).await?;
    let branch = r.copy_branch.clone().ok_or(ApiError::BadRequest)?;
    let topic = crate::workspace::topic(&state, r.topic_id).await?;
    let copy = snapshots::folder(&state, &r).await?;
    // Ni tour ni vérification dans la copie ou dans une session du topic.
    let topic_folder = snapshots::Folder {
        session_id: id.clone(),
        work: topic.dir.clone(),
        snap: snapshots::topic_repo(&state, topic.id),
    };
    sessions::ensure_folder_free(&state, &copy).await?;
    sessions::ensure_folder_free(&state, &topic_folder).await?;
    let _g1 = snapshots::lock(&state, &copy).await;
    let _g2 = snapshots::lock(&state, &topic_folder).await;
    let mut env = sandboxes::git_env(&state).await;
    env.push(("BRANCH".into(), branch.clone()));
    env.push(("MSG".into(), format!("EngineX : {}", state.masker().mask(&r.title))));
    let o = helper(
        &state,
        &id,
        vec![(copy.work.clone(), "COPY", true), (topic.dir.clone(), "TOPIC", true)],
        BRING,
        env,
        BRING_TIMEOUT,
    )
    .await?;
    Ok(Json(match o.code {
        0 => json!({"result": "merged", "branch": branch}),
        10 => json!({"result": "branch_only", "reason": "topic_dirty", "branch": branch}),
        11 => json!({"result": "branch_only", "reason": "conflict", "branch": branch}),
        _ => return Err(ApiError::CopyFailed(state.masker().mask(o.stderr.trim()).into_owned())),
    }))
}
