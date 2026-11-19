//! Sondes des binaires llama-server et nvidia-smi. `DEVICE_LINE`,
//! `parse_list_devices` et `parse_nvidia_smi` sont portés de Loki
//! (`rust/loki-rs/src/backends/devices.rs`, `rust/loki-rs/src/sysinfo.rs`,
//! MIT, voir NOTICE.md).

use std::{path::Path, process::Stdio, sync::LazyLock, time::Duration};

use regex::Regex;
use serde::Serialize;

pub const CMD_TIMEOUT: Duration = Duration::from_secs(10);

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

// `\s` du Go : blancs ASCII seulement (comme Loki).
pub static DEVICE_LINE: LazyLock<Regex> = LazyLock::new(|| {
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
///
/// `lib_dir` : dossier des bibliothèques d'un binaire llama.cpp ; il remplace
/// le `LD_LIBRARY_PATH` hérité du conteneur (`/app`), pour qu'un moteur
/// installé ne charge jamais les bibliothèques de l'image. `None` pour les
/// autres commandes (nvidia-smi).
pub async fn run(
    bin: &Path,
    args: &[&str],
    merge_stderr: bool,
    lib_dir: Option<&Path>,
) -> Result<(i32, String), String> {
    let mut cmd = tokio::process::Command::new(bin);
    if let Some(dir) = lib_dir {
        cmd.env("LD_LIBRARY_PATH", dir);
    }
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

/// Dossier des bibliothèques d'un binaire llama.cpp : le sien.
pub fn lib_dir(bin: &Path) -> Option<&Path> {
    bin.parent().filter(|p| !p.as_os_str().is_empty())
}

pub async fn read_llama(bin: &Path) -> LlamaInfo {
    let mut info = LlamaInfo {
        bin: bin.display().to_string(),
        version: None,
        devices: Vec::new(),
        error: None,
    };
    match run(bin, &["--version"], true, lib_dir(bin)).await {
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
    match run(bin, &["--list-devices"], true, lib_dir(bin)).await {
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

pub async fn read_gpus(bin: &Path) -> Result<Vec<Gpu>, String> {
    let (code, out) = run(
        bin,
        &[
            "--query-gpu=index,name,memory.total,memory.used,temperature.gpu,driver_version",
            "--format=csv,noheader,nounits",
        ],
        false,
        None,
    )
    .await?;
    if code != 0 {
        return Err(format!("exit {code}"));
    }
    Ok(parse_nvidia_smi(&out))
}

/// `llama-server --help` (sortie standard et erreur).
pub async fn help(bin: &Path) -> Result<String, String> {
    run(bin, &["--help"], true, lib_dir(bin)).await.map(|(_, out)| out)
}

/// L'aide propose-t-elle le placement automatique (`--fit` et `--fit-target`) ?
pub fn has_fit(help: &str) -> bool {
    has_flag(help, "--fit") && help.contains("--fit-target")
}

fn has_flag(help: &str, flag: &str) -> bool {
    help.split(|c: char| c.is_whitespace() || c == ',').any(|w| w == flag)
}

/// Options d'accélération connues du binaire (un moteur ancien, après un
/// retour arrière, peut ne pas les avoir : elles sont alors omises).
pub fn caps(help: &str) -> crate::args::Caps {
    crate::args::Caps {
        fit: has_fit(help),
        spec: has_flag(help, "--spec-type") && help.contains("draft-mtp") && has_flag(help, "--spec-draft-n-max"),
        moe: has_flag(help, "--cpu-moe") && has_flag(help, "--n-cpu-moe"),
        budget: has_flag(help, "--reasoning-budget-message"),
    }
}

/// Numéro de build tiré de la version : `1.0.0-dev (build 12464, commit
/// d3c3e242c)` (b12464, relevé sur le binaire officiel) ou l'ancienne forme
/// `12464 (d3c3e242)` → `b12464`.
pub fn build_of(version: &str) -> Option<String> {
    let digits = |s: &str| -> String { s.chars().take_while(|c| c.is_ascii_digit()).collect() };
    let n = match version.find("build ") {
        Some(i) => digits(&version[i + "build ".len()..]),
        None => digits(version),
    };
    (!n.is_empty()).then(|| format!("b{n}"))
}

/// `GET /props` du llama-server lancé par EngineX.
pub async fn props(client: &reqwest::Client, port: u16) -> Result<serde_json::Value, String> {
    let res = client
        .get(format!("http://127.0.0.1:{port}/props"))
        .timeout(CMD_TIMEOUT)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("HTTP {}", res.status().as_u16()));
    }
    res.json().await.map_err(|e| e.to_string())
}

/// Le gabarit de discussion sait-il rendre un appel d'outil ? (`/apply-template`
/// avec un appel `get_weather` : OK si le rendu le contient.)
pub async fn tools_probe(client: &reqwest::Client, port: u16) -> Result<bool, String> {
    let body = serde_json::json!({
        "messages": [
            {"role": "user", "content": "Quel temps fait-il à Nancy ?"},
            {"role": "assistant", "content": "", "tool_calls": [{
                "id": "call_1", "type": "function",
                "function": {"name": "get_weather", "arguments": "{\"city\":\"Nancy\"}"}
            }]},
            {"role": "tool", "tool_call_id": "call_1", "content": "12 °C"}
        ],
        "tools": [{"type": "function", "function": {
            "name": "get_weather", "description": "Météo d'une ville",
            "parameters": {"type": "object", "properties": {"city": {"type": "string"}}, "required": ["city"]}
        }}]
    });
    let res = client
        .post(format!("http://127.0.0.1:{port}/apply-template"))
        .json(&body)
        .timeout(CMD_TIMEOUT)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!("HTTP {}", res.status().as_u16()));
    }
    let v: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
    Ok(v.get("prompt")
        .and_then(|p| p.as_str())
        .is_some_and(|p| p.contains("get_weather")))
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
    fn fit_and_build() {
        assert!(has_fit(
            "  -fit, --fit [on|off]   whether...\n  -fitt, --fit-target MiB0,MiB1\n"
        ));
        assert!(!has_fit("  --fit-target only\n"));
        assert!(!has_fit("-ngl N\n"));
        assert_eq!(build_of("12464 (d3c3e242)").as_deref(), Some("b12464"));
        assert_eq!(
            build_of("1.0.0-dev (build 12464, commit d3c3e242c)").as_deref(),
            Some("b12464")
        );
        assert_eq!(build_of("inconnue"), None);
    }

    #[test]
    fn acceleration_caps_from_help() {
        // Lignes relevées dans `llama-server --help` de b12464.
        let help = "-cmoe, --cpu-moe                        keep all Mixture of Experts (MoE) weights in the CPU\n\
                    -ncmoe, --n-cpu-moe N                   keep the Mixture of Experts (MoE) weights\n\
                    --spec-draft-n-max N                    number of tokens to draft\n\
                    --spec-type none,draft-simple,draft-eagle3,draft-mtp,ngram-mod\n";
        let c = caps(help);
        assert!(c.spec && c.moe && !c.fit);
        let old = caps("--draft-max N\n-ngl N\n");
        assert!(!old.spec && !old.moe);
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
