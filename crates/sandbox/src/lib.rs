//! Sandboxes de session : un `codex app-server` par session, dans un
//! conteneur Docker (`docker`, production) ou en processus local (`local`,
//! développement et tests, sans isolation).

pub mod docker;
pub mod local;
pub mod spec;

use std::{path::PathBuf, time::Duration};

/// Dossiers de la session vus par EngineX (sous son dossier de données).
#[derive(Clone, Debug)]
pub struct Mounts {
    /// Dossier de travail (topic, ou copie séparée) → `/workspace`.
    pub work: PathBuf,
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

/// Conteneur d'aide : un script `sh -c` fixe, qui ne voit que les dossiers de
/// `binds` (voir `spec::helper_spec`). Sans réseau sauf `network` (fetch et
/// push de la page Git, dans un dépôt nu propre au conteneur).
#[derive(Clone, Debug)]
pub struct HelperSpec {
    pub session_id: String,
    pub owner: String,
    /// (chemin vu par EngineX, variable d'environnement, écriture permise).
    /// Docker monte le dossier sous `/mnt/<variable en minuscules>` ; le
    /// backend local passe le chemin tel quel. La variable donne le chemin au
    /// script.
    pub binds: Vec<(PathBuf, &'static str, bool)>,
    pub script: &'static str,
    pub env: Vec<(String, String)>,
    /// Réseau des sandboxes au lieu de `none`.
    pub network: bool,
}

/// Résultat d'un conteneur d'aide.
#[derive(Clone, Debug)]
pub struct HelperOutcome {
    pub code: i64,
    pub stdout: Vec<u8>,
    /// 200 dernières lignes de stderr.
    pub stderr: String,
}

/// Taille maximale du stdout d'un conteneur d'aide.
pub const HELPER_STDOUT_MAX: usize = 8 << 20;

/// Erreur rendue quand le stdout dépasse `HELPER_STDOUT_MAX`.
pub const OUTPUT_TOO_LARGE: &str = "output_too_large";

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

    /// Lance un conteneur d'aide et attend sa fin (supprimé dans tous les cas).
    pub async fn run_helper(&self, spec: &HelperSpec, timeout: Duration) -> anyhow::Result<HelperOutcome> {
        match self {
            Self::Local(b) => b.run_helper(spec, timeout).await,
            Self::Docker(b) => b.run_helper(spec, timeout).await,
        }
    }

    /// Supprime les conteneurs d'aide restés d'un arrêt brutal.
    pub async fn remove_helpers(&self, owner: &str) -> anyhow::Result<()> {
        match self {
            Self::Local(_) => Ok(()),
            Self::Docker(b) => b.remove_helpers(owner).await,
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
