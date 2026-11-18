//! Moteurs llama.cpp tirés des images officielles (`ghcr.io/ggml-org/llama.cpp`) :
//! dernière version publiée, téléchargement des couches vérifiées par leur
//! digest, extraction des seuls fichiers `app/*`.

use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

pub const REPO: &str = "ggml-org/llama.cpp";
/// Une couche plus grosse est le runtime CUDA de l'image, pas le moteur.
pub const MAX_LAYER: u64 = 1 << 30;
/// Fichiers sans lesquels un moteur n'est pas installé (préfixes).
pub const REQUIRED: [&str; 6] = [
    "llama-server",
    "libllama.so",
    "libggml.so",
    "libggml-base.so",
    "libggml-cuda.so",
    "libmtmd.so",
];

const ACCEPT: &str = "application/vnd.oci.image.index.v1+json, application/vnd.oci.image.manifest.v1+json, \
    application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.docker.distribution.manifest.v2+json";
const VERSION: &str = "org.opencontainers.image.version";

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum UpdateError {
    #[error("réseau : {0}")]
    Network(String),
    #[error("version introuvable")]
    VersionNotFound,
    #[error("couche corrompue (digest différent)")]
    DigestMismatch,
    #[error("moteur incomplet, il manque : {0:?}")]
    Incomplete(Vec<String>),
    #[error("lien refusé : {0}")]
    BadLink(String),
    #[error("test échoué : {0}")]
    TestFailed(String),
    #[error("disque : {0}")]
    Io(String),
}

impl UpdateError {
    /// Code rangé dans `engine_versions.last_test` et montré par l'UI.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Network(_) => "network",
            Self::VersionNotFound => "version_not_found",
            Self::DigestMismatch => "digest_mismatch",
            Self::Incomplete(_) => "engine_incomplete",
            Self::BadLink(_) => "bad_link",
            Self::TestFailed(_) => "test_failed",
            Self::Io(_) => "io",
        }
    }
}

impl From<reqwest::Error> for UpdateError {
    fn from(e: reqwest::Error) -> Self {
        Self::Network(e.to_string())
    }
}

impl From<std::io::Error> for UpdateError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Layer {
    pub digest: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    /// Tag figé, ex. `server-cuda-b12464`.
    pub tag: String,
    /// Build, ex. `b12464`.
    pub build: String,
    pub manifest_digest: String,
    pub layers: Vec<Layer>,
}

#[derive(Clone)]
pub struct Registry {
    client: reqwest::Client,
    base: String,
}

impl Registry {
    pub fn new(client: reqwest::Client, base: impl Into<String>) -> Self {
        Self {
            client,
            base: base.into().trim_end_matches('/').to_string(),
        }
    }

    async fn token(&self) -> Result<String, UpdateError> {
        let host = self.base.split("://").nth(1).unwrap_or(&self.base).to_string();
        let url = format!("{}/token?scope=repository:{REPO}:pull&service={host}", self.base);
        let v: serde_json::Value = self
            .client
            .get(url)
            .timeout(Duration::from_secs(30))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        v.get("token")
            .and_then(|t| t.as_str())
            .map(str::to_string)
            .ok_or_else(|| UpdateError::Network("jeton absent".into()))
    }

    async fn manifest(&self, token: &str, reference: &str) -> Result<Option<serde_json::Value>, UpdateError> {
        let res = self
            .client
            .get(format!("{}/v2/{REPO}/manifests/{reference}", self.base))
            .bearer_auth(token)
            .header(reqwest::header::ACCEPT, ACCEPT)
            .timeout(Duration::from_secs(30))
            .send()
            .await?;
        if res.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        Ok(Some(res.error_for_status()?.json().await?))
    }

    /// Dernière version publiée sous le tag mouvant `variant` (ex. `server-cuda`).
    pub async fn latest(&self, variant: &str) -> Result<Release, UpdateError> {
        let token = self.token().await?;
        let top = self
            .manifest(&token, variant)
            .await?
            .ok_or(UpdateError::VersionNotFound)?;
        let version_of = |v: &serde_json::Value| {
            v.pointer("/annotations")
                .and_then(|a| a.get(VERSION))
                .and_then(|x| x.as_str())
                .map(str::to_string)
        };
        let (manifest, digest) = match top.get("manifests").and_then(|m| m.as_array()) {
            Some(list) => {
                let d = list
                    .iter()
                    .find(|m| {
                        m.pointer("/platform/os").and_then(|x| x.as_str()) == Some("linux")
                            && m.pointer("/platform/architecture").and_then(|x| x.as_str()) == Some("amd64")
                    })
                    .and_then(|m| m.get("digest"))
                    .and_then(|x| x.as_str())
                    .ok_or(UpdateError::VersionNotFound)?
                    .to_string();
                (self.manifest(&token, &d).await?.ok_or(UpdateError::VersionNotFound)?, d)
            }
            None => (top.clone(), String::new()),
        };
        let build = version_of(&top)
            .or_else(|| version_of(&manifest))
            .ok_or(UpdateError::VersionNotFound)?;
        if !build.starts_with('b') || !build[1..].chars().all(|c| c.is_ascii_digit()) {
            return Err(UpdateError::VersionNotFound);
        }
        let tag = format!("{variant}-{build}");
        if self.manifest(&token, &tag).await?.is_none() {
            return Err(UpdateError::VersionNotFound);
        }
        let layers = manifest
            .get("layers")
            .and_then(|l| l.as_array())
            .ok_or(UpdateError::VersionNotFound)?
            .iter()
            .filter_map(|l| {
                Some(Layer {
                    digest: l.get("digest")?.as_str()?.to_string(),
                    size: l.get("size")?.as_u64()?,
                })
            })
            .collect();
        Ok(Release {
            tag,
            build,
            manifest_digest: digest,
            layers,
        })
    }

    /// Télécharge une couche dans `dest`, en vérifiant son digest.
    async fn blob(
        &self,
        token: &str,
        layer: &Layer,
        dest: &Path,
        done: &mut u64,
        total: u64,
        progress: &(dyn Fn(u64, u64) + Send + Sync),
    ) -> Result<(), UpdateError> {
        let hex = layer
            .digest
            .strip_prefix("sha256:")
            .ok_or(UpdateError::DigestMismatch)?;
        let res = self
            .client
            .get(format!("{}/v2/{REPO}/blobs/{}", self.base, layer.digest))
            .bearer_auth(token)
            .send()
            .await?
            .error_for_status()?;
        let mut f = tokio::fs::File::create(dest).await?;
        let mut h = Sha256::new();
        let mut stream = res.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let b = chunk?;
            h.update(&b);
            f.write_all(&b).await?;
            *done += b.len() as u64;
            progress(*done, total);
        }
        f.flush().await?;
        let got: String = h.finalize().iter().map(|x| format!("{x:02x}")).collect();
        if got != hex {
            return Err(UpdateError::DigestMismatch);
        }
        Ok(())
    }

    /// Installe `rel` dans `<engines_dir>/<tag>/` et rend le chemin de son
    /// llama-server. En cas d'échec, rien ne reste sur le disque.
    pub async fn install(
        &self,
        rel: &Release,
        engines_dir: &Path,
        progress: &(dyn Fn(u64, u64) + Send + Sync),
    ) -> Result<PathBuf, UpdateError> {
        let part = engines_dir.join(format!("{}.part", rel.tag));
        let layers_dir = engines_dir.join(format!("{}.layers", rel.tag));
        let _ = tokio::fs::remove_dir_all(&part).await;
        let _ = tokio::fs::remove_dir_all(&layers_dir).await;
        tokio::fs::create_dir_all(&part).await?;
        tokio::fs::create_dir_all(&layers_dir).await?;
        let res = self.install_into(rel, &part, &layers_dir, progress).await;
        let _ = tokio::fs::remove_dir_all(&layers_dir).await;
        if let Err(e) = res {
            let _ = tokio::fs::remove_dir_all(&part).await;
            return Err(e);
        }
        let dest = engines_dir.join(&rel.tag);
        let _ = tokio::fs::remove_dir_all(&dest).await;
        tokio::fs::rename(&part, &dest).await?;
        Ok(dest.join("llama-server"))
    }

    async fn install_into(
        &self,
        rel: &Release,
        part: &Path,
        layers_dir: &Path,
        progress: &(dyn Fn(u64, u64) + Send + Sync),
    ) -> Result<(), UpdateError> {
        let token = self.token().await?;
        let wanted: Vec<&Layer> = rel.layers.iter().filter(|l| l.size < MAX_LAYER).collect();
        let total = wanted.iter().map(|l| l.size).sum();
        let mut done = 0;
        let mut files = Vec::new();
        for (i, l) in wanted.iter().enumerate() {
            let f = layers_dir.join(format!("{i}.tar.gz"));
            self.blob(&token, l, &f, &mut done, total, progress).await?;
            files.push(f);
        }
        let part = part.to_path_buf();
        tokio::task::spawn_blocking(move || -> Result<(), UpdateError> {
            // De la plus basse à la plus haute : la plus haute fait foi.
            for f in files {
                apply_layer(std::fs::File::open(f)?, &part)?;
            }
            check_complete(&part)
        })
        .await
        .map_err(|e| UpdateError::Io(e.to_string()))?
    }
}

/// Nom de fichier d'une entrée directe de `app/` (`None` sinon).
fn app_entry(path: &Path) -> Option<String> {
    let s = path.to_str()?.trim_start_matches("./").trim_start_matches('/');
    let name = s.strip_prefix("app/")?;
    (!name.is_empty() && !name.contains('/')).then(|| name.to_string())
}

fn simple_name(s: &str) -> bool {
    !s.is_empty() && !s.contains('/') && s != "." && s != ".."
}

/// Applique une couche tar.gz sur `dest` : fichiers et liens de `app/*` à plat.
pub fn apply_layer(tar_gz: impl Read, dest: &Path) -> Result<(), UpdateError> {
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(tar_gz));
    let mut hard: HashMap<String, String> = HashMap::new();
    for entry in ar.entries()? {
        let mut e = entry?;
        let Some(name) = app_entry(&e.path()?) else { continue };
        let target = dest.join(&name);
        if name == ".wh..wh..opq" {
            for f in std::fs::read_dir(dest)? {
                let p = f?.path();
                let _ = std::fs::remove_file(&p);
            }
            continue;
        }
        if let Some(gone) = name.strip_prefix(".wh.") {
            if simple_name(gone) {
                let _ = std::fs::remove_file(dest.join(gone));
            }
            continue;
        }
        let kind = e.header().entry_type();
        if kind.is_file() {
            let _ = std::fs::remove_file(&target);
            let mode = e.header().mode().unwrap_or(0o644) & 0o755;
            let mut out = std::fs::File::create(&target)?;
            std::io::copy(&mut e, &mut out)?;
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(mode | 0o400))?;
        } else if kind.is_symlink() {
            let link = e.link_name()?.ok_or_else(|| UpdateError::BadLink(name.clone()))?;
            let link = link.to_str().unwrap_or("").to_string();
            if !simple_name(&link) {
                return Err(UpdateError::BadLink(format!("{name} -> {link}")));
            }
            let _ = std::fs::remove_file(&target);
            std::os::unix::fs::symlink(&link, &target)?;
        } else if kind.is_hard_link() {
            let link = e.link_name()?.ok_or_else(|| UpdateError::BadLink(name.clone()))?;
            let src = app_entry(&link).ok_or_else(|| UpdateError::BadLink(format!("{name} -> {}", link.display())))?;
            hard.insert(name, src);
        }
    }
    for (name, src) in hard {
        let target = dest.join(&name);
        let _ = std::fs::remove_file(&target);
        std::fs::copy(dest.join(&src), &target).map_err(|_| UpdateError::BadLink(format!("{name} -> {src}")))?;
    }
    Ok(())
}

/// Vérifie le jeu de fichiers requis.
pub fn check_complete(dir: &Path) -> Result<(), UpdateError> {
    let names: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
        .collect();
    let missing: Vec<String> = REQUIRED
        .iter()
        .filter(|r| !names.iter().any(|n| n == *r || n.starts_with(&format!("{r}."))))
        .map(|r| r.to_string())
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(UpdateError::Incomplete(missing))
    }
}
