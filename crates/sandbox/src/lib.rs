//! Sandboxes de session : un `codex app-server` par session, dans un
//! conteneur Docker (`docker`, production) ou en processus local (`local`,
//! développement et tests, sans isolation).

pub mod docker;
pub mod local;
pub mod spec;

use std::path::PathBuf;

/// Dossiers de la session vus par EngineX (sous son dossier de données).
#[derive(Clone, Debug)]
pub struct Mounts {
    /// Dossier du topic → `/workspace`.
    pub topic: PathBuf,
    /// `<data>/sessions/<id>/codex` → `/home/dev/.codex` (CODEX_HOME).
    pub codex_home: PathBuf,
    /// `<data>/sessions/<id>/run` (jeton) → `/run/enginex`, lecture seule.
    pub run: PathBuf,
    /// `<data>/sessions/<id>/attachments` → `/attachments`, lecture seule.
    pub attachments: PathBuf,
    /// `<data>/cache/<slug du projet>` → `/cache`.
    pub cache: PathBuf,
}

/// Ce qu'il faut pour démarrer une sandbox.
#[derive(Clone, Debug)]
pub struct Spec {
    pub session_id: String,
    /// Id d'instance d'EngineX (label `enginex.owner`).
    pub owner: String,
    /// `<id du preset>:<updated_at>` (label `enginex.preset`).
    pub preset: String,
    pub mounts: Mounts,
    /// Variables d'environnement (secrets, clé d'API, extras).
    pub env: Vec<(String, String)>,
}

/// Sandbox démarrée.
#[derive(Clone, Debug)]
pub struct Handle {
    pub ws_url: String,
    pub readyz_url: String,
    /// Dossier de travail de Codex (`/workspace` ou le topic en local).
    pub cwd: String,
    /// Dossier des pièces jointes vu par Codex.
    pub attachments: String,
}

/// Sandbox trouvée par `list_owned`.
#[derive(Clone, Debug)]
pub struct Owned {
    pub session_id: String,
    pub preset: String,
}

/// Événement de démarrage (tirage d'image).
#[derive(Clone, Debug)]
pub enum Progress {
    Pulling { done: u64, total: u64 },
    Starting,
}

pub enum Backend {
    Local(local::Local),
    Docker(docker::Docker),
}

impl Backend {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Local(_) => "local",
            Self::Docker(_) => "docker",
        }
    }

    pub async fn start(&self, spec: &Spec, progress: &(dyn Fn(Progress) + Send + Sync)) -> anyhow::Result<Handle> {
        match self {
            Self::Local(b) => b.start(spec).await,
            Self::Docker(b) => b.start(spec, progress).await,
        }
    }

    /// Arrête et supprime la sandbox de la session (sans erreur si absente).
    pub async fn stop(&self, session_id: &str) -> anyhow::Result<()> {
        match self {
            Self::Local(b) => b.stop(session_id).await,
            Self::Docker(b) => b.stop(session_id).await,
        }
    }

    /// Sandboxes de cette instance encore présentes.
    pub async fn list_owned(&self, owner: &str) -> anyhow::Result<Vec<Owned>> {
        match self {
            Self::Local(b) => b.list_owned(owner).await,
            Self::Docker(b) => b.list_owned(owner).await,
        }
    }

    /// Dernières lignes du journal de la sandbox.
    pub async fn logs(&self, session_id: &str) -> String {
        match self {
            Self::Local(b) => b.logs(session_id).await,
            Self::Docker(b) => b.logs(session_id).await,
        }
    }
}

/// Garde les `n` dernières lignes de `s`.
pub fn tail(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}
