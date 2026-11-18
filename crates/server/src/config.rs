//! Configuration lue une fois au démarrage, dans l'environnement.
//! Les tests construisent `Config` directement.

use std::{net::SocketAddr, path::PathBuf};

#[derive(Clone, Debug)]
pub struct Config {
    pub bind: SocketAddr,
    pub data_dir: PathBuf,
    pub llama_bin: PathBuf,
    pub nvidia_smi: PathBuf,
    pub trust_proxy: bool,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let var = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
        let bind = var("ENGINEX_BIND", "0.0.0.0:8090");
        let trust = var("ENGINEX_TRUST_PROXY", "false").to_ascii_lowercase();
        Ok(Self {
            bind: bind
                .parse()
                .map_err(|e| anyhow::anyhow!("ENGINEX_BIND invalide ({bind}) : {e}"))?,
            data_dir: var("ENGINEX_DATA", "/data").into(),
            llama_bin: var("ENGINEX_LLAMA_BIN", "/app/llama-server").into(),
            nvidia_smi: var("ENGINEX_NVIDIA_SMI", "nvidia-smi").into(),
            trust_proxy: trust == "1" || trust == "true",
        })
    }

    /// Vérifie que le dossier de données est inscriptible (bind mount mal
    /// possédé : message qui donne l'uid réel).
    pub fn check_data_dir(&self) -> anyhow::Result<()> {
        let probe = self.data_dir.join(".write-test");
        let res = std::fs::create_dir_all(&self.data_dir)
            .and_then(|_| std::fs::write(&probe, b"ok"))
            .and_then(|_| std::fs::remove_file(&probe));
        if let Err(e) = res {
            // SAFETY: getuid/getgid n'ont aucune précondition.
            let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
            anyhow::bail!(
                "{} n'est pas inscriptible par l'uid {uid} : chown {uid}:{gid} sur le dossier monté, ou user: dans la stack ({e})",
                self.data_dir.display()
            );
        }
        Ok(())
    }
}
