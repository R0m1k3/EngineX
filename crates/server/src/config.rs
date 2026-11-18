//! Configuration lue une fois au démarrage, dans l'environnement.
//! Les tests construisent `Config` directement.

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};

use crate::state::READY_TIMEOUT;

/// Délais de l'API `/v1` et de la mise à jour automatique (les tests les
/// raccourcissent ; pas de variable d'environnement).
#[derive(Clone, Debug)]
pub struct Timings {
    /// Événement de maintien envoyé à Codex quand rien n'arrive.
    pub keepalive: Duration,
    /// Attente maximale de la porte de service par une requête.
    pub wait_limit: Duration,
    /// Attente du résultat de la sonde « outils » après une bascule.
    pub probe_wait: Duration,
    /// Lance la boucle de mise à jour automatique (les tests appellent `tick`).
    pub auto_loop: bool,
    pub auto: AutoUpdateTimings,
    pub sandbox: SandboxTimings,
}

#[derive(Clone, Debug)]
pub struct SandboxTimings {
    /// Attente de `/readyz` après le démarrage d'une sandbox.
    pub ready_wait: Duration,
    /// Délai d'une requête à l'app-server.
    pub rpc_timeout: Duration,
    /// Période du contrôle d'inactivité.
    pub idle_tick: Duration,
    /// Remplace `sandbox.idle_minutes` (tests).
    pub idle_override: Option<Duration>,
    /// Tirage de l'image devbox en fond (`None` : jamais).
    pub pull_every: Option<Duration>,
}

impl Default for SandboxTimings {
    fn default() -> Self {
        Self {
            ready_wait: Duration::from_secs(60),
            rpc_timeout: Duration::from_secs(30),
            idle_tick: Duration::from_secs(30),
            idle_override: None,
            pull_every: Some(Duration::from_secs(24 * 3600)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SandboxBackend {
    Docker,
    Local,
}

/// Sandboxes de session (variables `ENGINEX_SANDBOX*`, `ENGINEX_DOCKER_HOST`,
/// `ENGINEX_DEVBOX_IMAGE`, `ENGINEX_HOST_DATA`, `ENGINEX_CODEX_BIN`).
#[derive(Clone, Debug)]
pub struct SandboxConfig {
    pub backend: SandboxBackend,
    /// Binaire `codex` du backend local.
    pub codex_bin: PathBuf,
    pub docker_host: String,
    pub devbox_image: String,
    /// Dossier de données vu par l'hôte Docker (sinon découvert).
    pub host_data: Option<PathBuf>,
    pub network: String,
    /// URL de `/v1` vue depuis une sandbox.
    pub api_url: String,
    pub host_gateway: bool,
    /// Mémoire d'une sandbox (octets).
    pub memory: i64,
    /// Variables ajoutées à chaque sandbox (tests ; vide en production).
    pub extra_env: Vec<(String, String)>,
    /// `ENGINEX_RECORD_DIR` : enregistre les échanges du pont (contrat).
    pub record_dir: Option<PathBuf>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            backend: SandboxBackend::Local,
            codex_bin: "codex".into(),
            docker_host: "tcp://docker-proxy:2375".into(),
            devbox_image: "ghcr.io/r0m1k3/enginex-devbox:latest".into(),
            host_data: None,
            network: "enginex-sandbox".into(),
            api_url: "http://enginex:8090/v1".into(),
            host_gateway: false,
            memory: 8 << 30,
            extra_env: Vec::new(),
            record_dir: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AutoUpdateTimings {
    pub first_check: Duration,
    pub check_every: Duration,
    /// Durée sans requête `/v1` avant une mise en service.
    pub idle_for: Duration,
    pub tick: Duration,
}

impl Default for Timings {
    fn default() -> Self {
        Self {
            keepalive: Duration::from_secs(15),
            wait_limit: READY_TIMEOUT * 2 + Duration::from_secs(60),
            probe_wait: Duration::from_secs(30),
            auto_loop: true,
            auto: AutoUpdateTimings {
                first_check: Duration::from_secs(5 * 60),
                check_every: Duration::from_secs(6 * 3600),
                idle_for: Duration::from_secs(10 * 60),
                tick: Duration::from_secs(60),
            },
            sandbox: SandboxTimings::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    /// `ENGINEX_LLAMA_BIN` : impose le binaire (tests, dépannage). Absent : le
    /// moteur choisi dans Paramètres → Moteur, sinon celui de l'image.
    pub llama_bin: Option<PathBuf>,
    /// Binaire llama-server de l'image.
    pub image_bin: PathBuf,
    pub nvidia_smi: PathBuf,
    pub trust_proxy: bool,
    pub models_dir: PathBuf,
    /// Port local de llama-server (127.0.0.1 seulement).
    pub llama_port: u16,
    pub registry: String,
    pub hf_base: String,
    pub timings: Timings,
    pub sandbox: SandboxConfig,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let var = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
        let bind = var("ENGINEX_BIND", "0.0.0.0:8090");
        let trust = var("ENGINEX_TRUST_PROXY", "false").to_ascii_lowercase();
        let port = var("ENGINEX_LLAMA_PORT", "8081");
        Ok(Self {
            bind: bind
                .parse()
                .map_err(|e| anyhow::anyhow!("ENGINEX_BIND invalide ({bind}) : {e}"))?,
            data_dir: var("ENGINEX_DATA", "/data").into(),
            llama_bin: std::env::var_os("ENGINEX_LLAMA_BIN")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            image_bin: var("ENGINEX_IMAGE_LLAMA_BIN", "/app/llama-server").into(),
            nvidia_smi: var("ENGINEX_NVIDIA_SMI", "nvidia-smi").into(),
            trust_proxy: trust == "1" || trust == "true",
            models_dir: var("ENGINEX_MODELS", "/models").into(),
            llama_port: port
                .parse()
                .map_err(|e| anyhow::anyhow!("ENGINEX_LLAMA_PORT invalide ({port}) : {e}"))?,
            registry: var("ENGINEX_OCI_REGISTRY", "https://ghcr.io"),
            hf_base: var("ENGINEX_HF_BASE", "https://huggingface.co"),
            timings: Timings::default(),
            sandbox: sandbox_from_env()?,
        })
    }

    /// Dossier des moteurs llama.cpp installés.
    pub fn engines_dir(&self) -> PathBuf {
        self.data_dir.join("engines")
    }

    /// Vérifie que les dossiers de données et de modèles sont inscriptibles
    /// (bind mount mal possédé : message qui donne l'uid réel).
    pub fn check_data_dir(&self) -> anyhow::Result<()> {
        check_writable(&self.data_dir)?;
        check_writable(&self.models_dir)
    }
}

fn sandbox_from_env() -> anyhow::Result<SandboxConfig> {
    let d = SandboxConfig::default();
    let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let backend = match var("ENGINEX_SANDBOX").as_deref().unwrap_or("docker") {
        "docker" => SandboxBackend::Docker,
        "local" => SandboxBackend::Local,
        other => anyhow::bail!("ENGINEX_SANDBOX invalide ({other}) : docker ou local"),
    };
    let memory = match var("ENGINEX_SANDBOX_MEMORY") {
        Some(v) => parse_size(&v).ok_or_else(|| anyhow::anyhow!("ENGINEX_SANDBOX_MEMORY invalide ({v})"))?,
        None => d.memory,
    };
    Ok(SandboxConfig {
        backend,
        codex_bin: var("ENGINEX_CODEX_BIN").map(PathBuf::from).unwrap_or(d.codex_bin),
        docker_host: var("ENGINEX_DOCKER_HOST").unwrap_or(d.docker_host),
        devbox_image: var("ENGINEX_DEVBOX_IMAGE").unwrap_or(d.devbox_image),
        host_data: var("ENGINEX_HOST_DATA").map(PathBuf::from),
        network: var("ENGINEX_SANDBOX_NETWORK").unwrap_or(d.network),
        api_url: var("ENGINEX_SANDBOX_API_URL").unwrap_or(d.api_url),
        host_gateway: matches!(var("ENGINEX_SANDBOX_HOST_GATEWAY").as_deref(), Some("1" | "true")),
        memory,
        extra_env: Vec::new(),
        record_dir: var("ENGINEX_RECORD_DIR").map(PathBuf::from),
    })
}

/// `8g`, `512m`, `1073741824`.
fn parse_size(v: &str) -> Option<i64> {
    let v = v.trim().to_ascii_lowercase();
    let (n, mul) = match v.chars().last()? {
        'g' => (&v[..v.len() - 1], 1i64 << 30),
        'm' => (&v[..v.len() - 1], 1 << 20),
        'k' => (&v[..v.len() - 1], 1 << 10),
        _ => (&v[..], 1),
    };
    n.trim_end_matches('i').parse::<i64>().ok().map(|n| n * mul)
}

fn check_writable(dir: &Path) -> anyhow::Result<()> {
    let probe = dir.join(".write-test");
    let res = std::fs::create_dir_all(dir)
        .and_then(|_| std::fs::write(&probe, b"ok"))
        .and_then(|_| std::fs::remove_file(&probe));
    if let Err(e) = res {
        // SAFETY: getuid/getgid n'ont aucune précondition.
        let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
        anyhow::bail!(
            "{} n'est pas inscriptible par l'uid {uid} : chown {uid}:{gid} sur le dossier monté, ou user: dans la stack ({e})",
            dir.display()
        );
    }
    Ok(())
}
