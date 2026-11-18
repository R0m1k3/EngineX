//! Configuration lue une fois au démarrage, dans l'environnement.
//! Les tests construisent `Config` directement.

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

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
