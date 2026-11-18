//! Téléchargement d'un fichier avec reprise (`Range`), sha256 calculé en flux,
//! nouveaux essais et annulation. Le renommage du `.part` en fichier final est
//! fait par l'appelant, après succès.

use std::{
    path::Path,
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::hf::FileSpec;

/// Attentes entre deux essais (5 essais en plus du premier).
pub const DELAYS: [Duration; 5] = [
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
    Duration::from_secs(30),
];
const GIB: u64 = 1 << 30;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DownloadError {
    #[error("annulé")]
    Cancelled,
    #[error("sha256 différent de celui annoncé par Hugging Face")]
    Sha256Mismatch,
    #[error("HTTP {0}")]
    Status(u16),
    #[error("réseau : {0}")]
    Network(String),
    #[error("disque : {0}")]
    Io(String),
}

impl From<std::io::Error> for DownloadError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

/// Octets libres sur le système de fichiers de `dir`.
pub fn free_bytes(dir: &Path) -> std::io::Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(dir.as_os_str().as_bytes())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    // SAFETY: `c` est une chaîne C valide, `st` est initialisé par statvfs.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(st.f_bavail as u64 * st.f_frsize as u64)
}

/// Assez de place pour `remaining` octets, plus 1 Gio de marge ?
pub fn enough_space(free: u64, remaining: u64) -> bool {
    free >= remaining.saturating_add(GIB)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Relit un `.part` existant pour remettre le haché à jour.
async fn rehash(part: &Path, h: &mut Sha256) -> std::io::Result<u64> {
    let mut f = match tokio::fs::File::open(part).await {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut buf = vec![0u8; 1 << 20];
    let mut n = 0u64;
    loop {
        let r = f.read(&mut buf).await?;
        if r == 0 {
            return Ok(n);
        }
        h.update(&buf[..r]);
        n += r as u64;
    }
}

enum Attempt {
    Done,
    Retry(String),
}

pub async fn fetch(
    client: &reqwest::Client,
    url: &str,
    part: &Path,
    spec: &FileSpec,
    cancel: &CancellationToken,
    progress: &(dyn Fn(u64) + Send + Sync),
) -> Result<(), DownloadError> {
    fetch_with(client, url, part, spec, cancel, progress, &DELAYS).await
}

/// `fetch` avec des attentes choisies (tests).
pub async fn fetch_with(
    client: &reqwest::Client,
    url: &str,
    part: &Path,
    spec: &FileSpec,
    cancel: &CancellationToken,
    progress: &(dyn Fn(u64) + Send + Sync),
    delays: &[Duration],
) -> Result<(), DownloadError> {
    let mut hasher = Sha256::new();
    let mut have = rehash(part, &mut hasher).await?;
    if have > spec.size {
        tokio::fs::write(part, b"").await?;
        hasher = Sha256::new();
        have = 0;
    }
    progress(have);
    let mut tries = 0usize;
    while have < spec.size {
        match attempt(client, url, part, spec, cancel, progress, &mut hasher, &mut have).await? {
            Attempt::Done => {}
            Attempt::Retry(why) => {
                let Some(d) = delays.get(tries) else {
                    return Err(DownloadError::Network(why));
                };
                tries += 1;
                tracing::warn!("téléchargement {} : {why}, nouvel essai dans {d:?}", spec.name);
                tokio::select! {
                    _ = cancel.cancelled() => return Err(DownloadError::Cancelled),
                    _ = tokio::time::sleep(*d) => {}
                }
            }
        }
    }
    if hex(&hasher.finalize()) != spec.sha256.to_ascii_lowercase() {
        return Err(DownloadError::Sha256Mismatch);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn attempt(
    client: &reqwest::Client,
    url: &str,
    part: &Path,
    spec: &FileSpec,
    cancel: &CancellationToken,
    progress: &(dyn Fn(u64) + Send + Sync),
    hasher: &mut Sha256,
    have: &mut u64,
) -> Result<Attempt, DownloadError> {
    let mut req = client.get(url).header(reqwest::header::ACCEPT_ENCODING, "identity");
    if *have > 0 {
        req = req.header(reqwest::header::RANGE, format!("bytes={}-", *have));
    }
    let res = tokio::select! {
        _ = cancel.cancelled() => return Err(DownloadError::Cancelled),
        r = req.send() => r,
    };
    let res = match res {
        Ok(r) => r,
        Err(e) => return Ok(Attempt::Retry(e.to_string())),
    };
    let status = res.status().as_u16();
    let append = match status {
        206 => true,
        200 => false,
        // Plage refusée : on repart de zéro.
        416 => {
            tokio::fs::write(part, b"").await?;
            *hasher = Sha256::new();
            *have = 0;
            return Ok(Attempt::Retry("HTTP 416".into()));
        }
        429 | 500..=599 => return Ok(Attempt::Retry(format!("HTTP {status}"))),
        s => return Err(DownloadError::Status(s)),
    };
    if !append && *have > 0 {
        // Le serveur ignore la plage : tout recommencer.
        *hasher = Sha256::new();
        *have = 0;
    }
    let mut f = tokio::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(append)
        .truncate(!append)
        .open(part)
        .await?;
    let mut stream = res.bytes_stream();
    let mut last = Instant::now();
    loop {
        let chunk = tokio::select! {
            _ = cancel.cancelled() => {
                f.flush().await?;
                return Err(DownloadError::Cancelled);
            }
            c = stream.next() => c,
        };
        match chunk {
            None => break,
            Some(Err(e)) => {
                f.flush().await?;
                return Ok(Attempt::Retry(e.to_string()));
            }
            Some(Ok(b)) => {
                if *have + b.len() as u64 > spec.size {
                    f.flush().await?;
                    return Err(DownloadError::Io(
                        "le serveur envoie plus que la taille annoncée".into(),
                    ));
                }
                f.write_all(&b).await?;
                hasher.update(&b);
                *have += b.len() as u64;
                if last.elapsed() >= Duration::from_millis(250) {
                    progress(*have);
                    last = Instant::now();
                }
            }
        }
    }
    f.flush().await?;
    progress(*have);
    if *have < spec.size {
        return Ok(Attempt::Retry("connexion coupée".into()));
    }
    Ok(Attempt::Done)
}
