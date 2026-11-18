//! Client Hugging Face : recherche, fichiers GGUF d'un dépôt groupés par
//! quantification, en-tête GGUF lu par Range, README et
//! `generation_config.json`.

use std::{collections::BTreeMap, sync::LazyLock, time::Duration};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::gguf::{self, GgufError, GgufMeta};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum HubError {
    #[error("réseau : {0}")]
    Network(String),
    #[error("HTTP {0}")]
    Status(u16),
    #[error("dépôt à accès restreint")]
    Gated,
    #[error("données illisibles : {0}")]
    BadData(String),
}

impl From<reqwest::Error> for HubError {
    fn from(e: reqwest::Error) -> Self {
        match e.status() {
            Some(s) => Self::Status(s.as_u16()),
            None => Self::Network(e.to_string()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSpec {
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuantGroup {
    pub quant: String,
    /// Tranches dans l'ordre (une seule pour un fichier non découpé).
    pub files: Vec<FileSpec>,
    pub total_size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoInfo {
    pub repo: String,
    pub gated: bool,
    pub base_model: Option<String>,
    pub groups: Vec<QuantGroup>,
    pub mmproj: Vec<FileSpec>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoHit {
    pub repo: String,
    pub downloads: u64,
    pub likes: u64,
    pub updated: Option<String>,
    pub gated: bool,
}

/// Premier essai de lecture d'en-tête, puis second essai plus large.
const HEADER_STEPS: [u64; 2] = [16 << 20, 64 << 20];

static SHARD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"-(\d{5})-of-(\d{5})\.gguf$").expect("motif"));
static QUANT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:UD-)?(?:IQ\d_[A-Z0-9_]+|Q\d(?:_[A-Z0-9]+)*|BF16|F16|F32|MXFP4(?:_[A-Z0-9]+)*)").expect("motif")
});

/// `gated` de l'API : `false`, `"auto"` ou `"manual"`.
fn is_gated(v: Option<&serde_json::Value>) -> bool {
    match v {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => !s.is_empty() && s != "false",
        _ => false,
    }
}

/// Quantification lue dans un nom de fichier (dernier motif trouvé).
pub fn quant_of(file: &str) -> String {
    let name = file.rsplit('/').next().unwrap_or(file);
    let stem = SHARD.replace(name, ".gguf");
    let stem = stem.trim_end_matches(".gguf");
    QUANT
        .find_iter(stem)
        .last()
        .map(|m| m.as_str().to_ascii_uppercase())
        .unwrap_or_else(|| stem.to_string())
}

#[derive(Deserialize)]
struct TreeEntry {
    #[serde(rename = "type")]
    kind: String,
    path: String,
    #[serde(default)]
    size: u64,
    lfs: Option<Lfs>,
}

#[derive(Deserialize)]
struct Lfs {
    oid: String,
    size: u64,
}

/// Regroupe les GGUF d'un arbre de fichiers : quantifications et mmproj.
fn group(entries: Vec<TreeEntry>) -> (Vec<QuantGroup>, Vec<FileSpec>) {
    let mut groups: BTreeMap<String, Vec<FileSpec>> = BTreeMap::new();
    let mut mmproj = Vec::new();
    for e in entries {
        if e.kind != "file" || !e.path.to_ascii_lowercase().ends_with(".gguf") {
            continue;
        }
        let Some(lfs) = e.lfs else { continue };
        let spec = FileSpec {
            size: if lfs.size > 0 { lfs.size } else { e.size },
            sha256: lfs.oid,
            name: e.path,
        };
        if spec.name.to_ascii_lowercase().contains("mmproj") {
            mmproj.push(spec);
        } else {
            groups.entry(quant_of(&spec.name)).or_default().push(spec);
        }
    }
    let groups = groups
        .into_iter()
        .map(|(quant, mut files)| {
            files.sort_by(|a, b| a.name.cmp(&b.name));
            QuantGroup {
                total_size: files.iter().map(|f| f.size).sum(),
                quant,
                files,
            }
        })
        .collect();
    // Q8_0 d'abord, puis F16 / BF16, puis le reste.
    mmproj.sort_by_key(|f: &FileSpec| {
        let q = quant_of(&f.name);
        (
            match q.as_str() {
                "Q8_0" => 0,
                "F16" => 1,
                "BF16" => 2,
                _ => 3,
            },
            f.name.clone(),
        )
    });
    (groups, mmproj)
}

#[derive(Clone)]
pub struct Hf {
    client: reqwest::Client,
    base: String,
}

impl Hf {
    pub fn new(client: reqwest::Client, base: impl Into<String>) -> Self {
        Self {
            client,
            base: base.into().trim_end_matches('/').to_string(),
        }
    }

    pub fn file_url(&self, repo: &str, file: &str) -> String {
        format!("{}/{repo}/resolve/main/{file}", self.base)
    }

    pub fn raw_url(&self, repo: &str, file: &str) -> String {
        format!("{}/{repo}/raw/main/{file}", self.base)
    }

    async fn json(&self, url: &str) -> Result<serde_json::Value, HubError> {
        let res = self.client.get(url).timeout(Duration::from_secs(30)).send().await?;
        if !res.status().is_success() {
            return Err(HubError::Status(res.status().as_u16()));
        }
        res.json().await.map_err(|e| HubError::BadData(e.to_string()))
    }

    pub async fn search(&self, q: &str) -> Result<Vec<RepoHit>, HubError> {
        let url = format!(
            "{}/api/models?filter=gguf&sort=downloads&direction=-1&limit=30&expand[]=downloads&expand[]=likes&expand[]=lastModified&expand[]=gated",
            self.base
        );
        let res = self
            .client
            .get(url)
            .query(&[("search", q)])
            .timeout(Duration::from_secs(30))
            .send()
            .await?;
        if !res.status().is_success() {
            return Err(HubError::Status(res.status().as_u16()));
        }
        let v: Vec<serde_json::Value> = res.json().await.map_err(|e| HubError::BadData(e.to_string()))?;
        Ok(v.iter()
            .filter_map(|m| {
                Some(RepoHit {
                    repo: m.get("id")?.as_str()?.to_string(),
                    downloads: m.get("downloads").and_then(|x| x.as_u64()).unwrap_or(0),
                    likes: m.get("likes").and_then(|x| x.as_u64()).unwrap_or(0),
                    updated: m.get("lastModified").and_then(|x| x.as_str()).map(str::to_string),
                    gated: is_gated(m.get("gated")),
                })
            })
            .collect())
    }

    pub async fn repo(&self, repo: &str) -> Result<RepoInfo, HubError> {
        let info = self.json(&format!("{}/api/models/{repo}", self.base)).await?;
        let base_model = base_model_of(&info);
        let tree = self
            .json(&format!("{}/api/models/{repo}/tree/main?recursive=1", self.base))
            .await?;
        let entries: Vec<TreeEntry> = serde_json::from_value(tree).map_err(|e| HubError::BadData(e.to_string()))?;
        let (groups, mmproj) = group(entries);
        Ok(RepoInfo {
            repo: repo.to_string(),
            gated: is_gated(info.get("gated")),
            base_model,
            groups,
            mmproj,
        })
    }

    /// Métadonnées GGUF d'un fichier distant, sans le télécharger.
    pub async fn header(&self, repo: &str, file: &str) -> Result<GgufMeta, HubError> {
        let url = self.file_url(repo, file);
        for (i, n) in HEADER_STEPS.iter().enumerate() {
            let res = self
                .client
                .get(&url)
                .header(reqwest::header::RANGE, format!("bytes=0-{}", n - 1))
                .header(reqwest::header::ACCEPT_ENCODING, "identity")
                .timeout(Duration::from_secs(60))
                .send()
                .await?;
            match res.status().as_u16() {
                200 | 206 => {}
                401 | 403 => return Err(HubError::Gated),
                s => return Err(HubError::Status(s)),
            }
            let bytes = res.bytes().await.map_err(|e| HubError::Network(e.to_string()))?;
            match gguf::parse(&bytes[..]) {
                Ok(m) => return Ok(m),
                Err(GgufError::Truncated) if i + 1 < HEADER_STEPS.len() => continue,
                Err(e) => return Err(HubError::BadData(e.to_string())),
            }
        }
        Err(HubError::BadData("métadonnées trop grandes".into()))
    }

    async fn raw(&self, repo: &str, file: &str) -> Result<Option<String>, HubError> {
        let res = self
            .client
            .get(self.raw_url(repo, file))
            .timeout(Duration::from_secs(30))
            .send()
            .await?;
        match res.status().as_u16() {
            200 => Ok(Some(res.text().await.map_err(|e| HubError::Network(e.to_string()))?)),
            404 => Ok(None),
            401 | 403 => Err(HubError::Gated),
            s => Err(HubError::Status(s)),
        }
    }

    pub async fn readme(&self, repo: &str) -> Result<Option<String>, HubError> {
        self.raw(repo, "README.md").await
    }

    pub async fn generation_config(&self, repo: &str) -> Result<Option<serde_json::Value>, HubError> {
        match self.raw(repo, "generation_config.json").await? {
            Some(t) => serde_json::from_str(&t)
                .map(Some)
                .map_err(|e| HubError::BadData(e.to_string())),
            None => Ok(None),
        }
    }

    /// `cardData.base_model` d'un dépôt (`None` si absent).
    pub async fn base_model(&self, repo: &str) -> Result<Option<String>, HubError> {
        Ok(base_model_of(
            &self.json(&format!("{}/api/models/{repo}", self.base)).await?,
        ))
    }
}

/// `cardData.base_model` : chaîne, ou liste dont on prend le premier.
fn base_model_of(info: &serde_json::Value) -> Option<String> {
    match info.pointer("/cardData/base_model") {
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(serde_json::Value::Array(a)) => a.first().and_then(|x| x.as_str()).map(str::to_string),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quants() {
        assert_eq!(quant_of("Qwen3-0.6B-Q8_0.gguf"), "Q8_0");
        assert_eq!(quant_of("Qwen3-32B-UD-Q4_K_XL.gguf"), "UD-Q4_K_XL");
        assert_eq!(quant_of("Qwen3-32B-IQ4_XS.gguf"), "IQ4_XS");
        assert_eq!(quant_of("BF16/Qwen3-32B-BF16-00001-of-00002.gguf"), "BF16");
        assert_eq!(quant_of("model.gguf"), "model");
    }

    #[test]
    fn grouping() {
        let e = |p: &str, s: u64| TreeEntry {
            kind: "file".into(),
            path: p.into(),
            size: 0,
            lfs: Some(Lfs {
                oid: format!("sha-{p}"),
                size: s,
            }),
        };
        let (g, m) = group(vec![
            e("BF16/M-BF16-00002-of-00002.gguf", 5),
            e("BF16/M-BF16-00001-of-00002.gguf", 10),
            e("M-Q4_K_M.gguf", 3),
            e("mmproj-F16.gguf", 2),
            e("mmproj-Q8_0.gguf", 1),
            e("README.md", 1),
            TreeEntry {
                kind: "directory".into(),
                path: "BF16".into(),
                size: 0,
                lfs: None,
            },
        ]);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].quant, "BF16");
        assert_eq!(g[0].total_size, 15);
        assert!(g[0].files[0].name.ends_with("00001-of-00002.gguf"));
        assert_eq!(m[0].name, "mmproj-Q8_0.gguf");
    }
}
