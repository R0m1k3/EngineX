//! Page Système : version et périphériques vus par llama-server, GPU vus
//! par nvidia-smi. `parse_list_devices` et `parse_nvidia_smi` sont portés de
//! Loki (`rust/loki-rs/src/backends/devices.rs`, `rust/loki-rs/src/sysinfo.rs`,
//! MIT, voir NOTICE.md).

use std::{
    path::Path,
    process::Stdio,
    sync::LazyLock,
    time::{Duration, Instant},
};

use axum::{
    Json,
    extract::{Query, State},
};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::{error::ApiError, state::AppState};

const CMD_TIMEOUT: Duration = Duration::from_secs(10);
/// `--list-devices` initialise CUDA : lecture gardée 5 minutes.
const LLAMA_TTL: Duration = Duration::from_secs(300);
const GPU_TTL: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub total_mib: i64,
    pub free_mib: i64,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Gpu {
    pub index: i64,
    pub name: String,
    pub total_mib: i64,
    pub used_mib: i64,
    pub temp_c: i64,
    pub driver: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct LlamaInfo {
    pub bin: String,
    pub version: Option<String>,
    pub devices: Vec<Device>,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct SysCache {
    llama: Option<(Instant, LlamaInfo)>,
    gpus: Option<(Instant, Result<Vec<Gpu>, String>)>,
}

// `\s` du Go : blancs ASCII seulement (comme Loki).
static DEVICE_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        &r"^\s*([A-Za-z]+[0-9]+):\s*(.+?)\s*\(([0-9]+)\s*MiB,\s*([0-9]+)\s*MiB free\)\s*$"
            .replace(r"\s", r"[\t\n\f\r ]"),
    )
    .expect("motif")
});

pub fn parse_list_devices(out: &str) -> Vec<Device> {
    out.split('\n')
        .filter_map(|line| DEVICE_LINE.captures(line.trim_end_matches('\r')))
        .map(|m| Device {
            id: m[1].to_string(),
            name: m[2].to_string(),
            total_mib: m[3].parse().unwrap_or(0),
            free_mib: m[4].parse().unwrap_or(0),
        })
        .collect()
}

/// Première ligne `version: …` de `llama-server --version`.
pub fn parse_version(out: &str) -> Option<String> {
    out.lines()
        .map(|l| l.trim_end_matches('\r').trim())
        .find_map(|l| l.strip_prefix("version:"))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Sortie `index,name,memory.total,memory.used,temperature.gpu,driver_version`
/// en `csv,noheader,nounits` : exactement 6 champs (sinon la ligne est
/// ignorée), un nombre illisible (« [N/A] ») vaut 0.
pub fn parse_nvidia_smi(out: &str) -> Vec<Gpu> {
    let n = |s: &str| s.parse::<i64>().unwrap_or(0);
    out.trim()
        .split('\n')
        .filter_map(|line| {
            let p: Vec<&str> = line.trim_end_matches('\r').split(',').map(str::trim).collect();
            (p.len() == 6).then(|| Gpu {
                index: n(p[0]),
                name: p[1].to_string(),
                total_mib: n(p[2]),
                used_mib: n(p[3]),
                temp_c: n(p[4]),
                driver: p[5].to_string(),
            })
        })
        .collect()
}

/// Lance une commande bornée dans le temps. Renvoie le code de sortie et la
/// sortie standard (plus la sortie d'erreur si `merge_stderr`).
async fn run(bin: &Path, args: &[&str], merge_stderr: bool) -> Result<(i32, String), String> {
    let mut cmd = tokio::process::Command::new(bin);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(if merge_stderr { Stdio::piped() } else { Stdio::null() })
        .kill_on_drop(true);
    let out = match tokio::time::timeout(CMD_TIMEOUT, cmd.output()).await {
        Err(_) => return Err("timeout".into()),
        Ok(Err(e)) if e.kind() == std::io::ErrorKind::NotFound => return Err("not_found".into()),
        Ok(Err(e)) => return Err(e.to_string()),
        Ok(Ok(o)) => o,
    };
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    if merge_stderr {
        text.push('\n');
        text.push_str(&String::from_utf8_lossy(&out.stderr));
    }
    Ok((out.status.code().unwrap_or(-1), text))
}

async fn read_llama(bin: &Path) -> LlamaInfo {
    let mut info = LlamaInfo {
        bin: bin.display().to_string(),
        version: None,
        devices: Vec::new(),
        error: None,
    };
    match run(bin, &["--version"], true).await {
        Ok((code, out)) => {
            info.version = parse_version(&out);
            if info.version.is_none() && code != 0 {
                info.error = Some(format!("exit {code}"));
            }
        }
        Err(e) => {
            info.error = Some(e);
            return info;
        }
    }
    match run(bin, &["--list-devices"], true).await {
        Ok((code, out)) => {
            info.devices = parse_list_devices(&out);
            if info.devices.is_empty() && code != 0 && info.error.is_none() {
                info.error = Some(format!("exit {code}"));
            }
        }
        Err(e) => info.error = info.error.or(Some(e)),
    }
    info
}

async fn read_gpus(bin: &Path) -> Result<Vec<Gpu>, String> {
    let (code, out) = run(
        bin,
        &[
            "--query-gpu=index,name,memory.total,memory.used,temperature.gpu,driver_version",
            "--format=csv,noheader,nounits",
        ],
        false,
    )
    .await?;
    if code != 0 {
        return Err(format!("exit {code}"));
    }
    Ok(parse_nvidia_smi(&out))
}

#[derive(Deserialize)]
pub struct SystemQuery {
    refresh: Option<String>,
}

pub async fn system(
    State(state): State<AppState>,
    Query(q): Query<SystemQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let cfg = state.config();
    let mut cache = state.0.sys_cache.lock().await;
    if q.refresh.as_deref().is_some_and(|v| v == "1" || v == "true") {
        *cache = SysCache::default();
    }
    let now = Instant::now();
    if !cache
        .llama
        .as_ref()
        .is_some_and(|(t, _)| now.duration_since(*t) < LLAMA_TTL)
    {
        cache.llama = Some((now, read_llama(&cfg.llama_bin).await));
    }
    if !cache
        .gpus
        .as_ref()
        .is_some_and(|(t, _)| now.duration_since(*t) < GPU_TTL)
    {
        cache.gpus = Some((now, read_gpus(&cfg.nvidia_smi).await));
    }
    let llama = cache.llama.as_ref().map(|(_, l)| l.clone());
    let (gpus, gpus_error) = match cache.gpus.as_ref().map(|(_, g)| g.clone()) {
        Some(Ok(g)) => (g, None),
        Some(Err(e)) => (Vec::new(), Some(e)),
        None => (Vec::new(), None),
    };
    Ok(Json(serde_json::json!({
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_s": state.0.started.elapsed().as_secs(),
        "llama": llama,
        "gpus": gpus,
        "gpus_error": gpus_error,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_devices() {
        let out = "ggml_cuda_init: found 2 CUDA devices\r\nAvailable devices:\r\n  CUDA0: NVIDIA GeForce RTX 5060 Ti (16311 MiB, 15900 MiB free)\r\n  bruit\n  CUDA1: NVIDIA GeForce RTX 3060 (12288 MiB, 12000 MiB free)\n";
        let d = parse_list_devices(out);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].id, "CUDA0");
        assert_eq!(d[0].name, "NVIDIA GeForce RTX 5060 Ti");
        assert_eq!((d[1].total_mib, d[1].free_mib), (12288, 12000));
    }

    #[test]
    fn nvidia_smi() {
        let out = "\n0, NVIDIA GeForce RTX 5060 Ti, 16311, 400, 38, 595.84\r\nligne cassée\n1, NVIDIA GeForce RTX 3060, 12288, [N/A], [N/A], 595.84\n\n";
        let g = parse_nvidia_smi(out);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].temp_c, 38);
        assert_eq!(g[0].driver, "595.84");
        assert_eq!((g[1].used_mib, g[1].temp_c), (0, 0));
    }

    #[test]
    fn version() {
        assert_eq!(
            parse_version("load_backend: ...\nversion: 7000 (abcdef0)\nbuilt with gcc\n").as_deref(),
            Some("7000 (abcdef0)")
        );
        assert_eq!(parse_version("rien\n"), None);
    }
}
