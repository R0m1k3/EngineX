//! Superviseur : un seul llama-server à la fois, lancé par EngineX dans son
//! propre groupe de processus, écoutant sur 127.0.0.1. Suit le chargement,
//! garde les logs, rend la cause d'un échec, arrête proprement.

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use serde::Serialize;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::watch,
};

use crate::probe;

/// Lignes de log gardées en mémoire.
const LOG_LINES: usize = 2000;
/// Taille du fichier de log avant rotation.
const LOG_MAX: u64 = 20 << 20;
const POLL: Duration = Duration::from_millis(500);
const TERM_GRACE: Duration = Duration::from_secs(5);
const EXIT_WAIT: Duration = Duration::from_secs(10);
const VRAM_WAIT: Duration = Duration::from_secs(15);

#[derive(Clone, Debug)]
pub struct SupervisorConfig {
    pub port: u16,
    pub log_path: PathBuf,
    pub nvidia_smi: PathBuf,
    pub ready_timeout: Duration,
}

#[derive(Clone, Debug)]
pub struct LaunchSpec {
    pub bin: PathBuf,
    pub argv: Vec<String>,
    pub model_bytes: u64,
    pub preset_id: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum EngineState {
    Stopped,
    Starting {
        preset_id: i64,
        progress: u8,
    },
    Ready {
        preset_id: i64,
        pid: u32,
    },
    Failed {
        preset_id: i64,
        cause: String,
        last_lines: Vec<String>,
    },
    Stopping,
}

impl EngineState {
    pub fn preset_id(&self) -> Option<i64> {
        match self {
            Self::Starting { preset_id, .. } | Self::Ready { preset_id, .. } | Self::Failed { preset_id, .. } => {
                Some(*preset_id)
            }
            _ => None,
        }
    }
}

/// Cause lisible d'un échec, tirée des dernières lignes de log.
pub fn failure_cause(lines: &[String]) -> String {
    let all = lines.join("\n").to_ascii_lowercase();
    if all.contains("cudamalloc failed") || all.contains("out of memory") {
        "VRAM insuffisante".into()
    } else if all.contains("unknown model architecture") {
        "architecture non prise en charge par cette version de llama.cpp".into()
    } else if all.contains("failed to load model") || all.contains("error loading model") {
        "fichier du modèle illisible".into()
    } else if all.contains("segmentation fault") {
        "llama-server a planté (segmentation fault)".into()
    } else {
        "llama-server s'est arrêté".into()
    }
}

struct Running {
    pid: u32,
    child: tokio::process::Child,
}

struct Inner {
    cfg: SupervisorConfig,
    state: watch::Sender<EngineState>,
    running: tokio::sync::Mutex<Option<Running>>,
    logs: Mutex<VecDeque<String>>,
    /// Numéro du lancement en cours : les tâches d'un ancien lancement s'arrêtent.
    generation: AtomicU64,
    http: reqwest::Client,
}

#[derive(Clone)]
pub struct Supervisor(Arc<Inner>);

impl Supervisor {
    pub fn new(cfg: SupervisorConfig) -> Self {
        Self(Arc::new(Inner {
            cfg,
            state: watch::channel(EngineState::Stopped).0,
            running: tokio::sync::Mutex::new(None),
            logs: Mutex::new(VecDeque::with_capacity(LOG_LINES)),
            generation: AtomicU64::new(0),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .no_proxy()
                .build()
                .expect("client HTTP"),
        }))
    }

    pub fn state(&self) -> EngineState {
        self.0.state.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<EngineState> {
        self.0.state.subscribe()
    }

    pub fn logs(&self, tail: usize) -> Vec<String> {
        let l = self.0.logs.lock().expect("logs");
        l.iter().skip(l.len().saturating_sub(tail)).cloned().collect()
    }

    pub fn port(&self) -> u16 {
        self.0.cfg.port
    }

    /// Lance llama-server (après avoir arrêté le précédent). Rend la main dès
    /// que le processus tourne ; l'état passe ensuite seul à `Ready` ou `Failed`.
    pub async fn start(&self, spec: LaunchSpec) -> Result<(), String> {
        if self.0.running.lock().await.is_some() {
            self.stop(true).await;
        }
        let generation = self.0.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.0.logs.lock().expect("logs").clear();
        let mut cmd = tokio::process::Command::new(&spec.bin);
        cmd.args(&spec.argv)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        if let Some(dir) = probe::lib_dir(&spec.bin) {
            cmd.env("LD_LIBRARY_PATH", dir);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("lancement de {} : {e}", spec.bin.display()))?;
        let pid = child.id().ok_or("pid introuvable")?;
        self.log_line(format!("[enginex] {} {}", spec.bin.display(), spec.argv.join(" ")));
        if let Some(out) = child.stdout.take() {
            self.pump(out);
        }
        if let Some(err) = child.stderr.take() {
            self.pump(err);
        }
        *self.0.running.lock().await = Some(Running { pid, child });
        self.0.state.send_replace(EngineState::Starting {
            preset_id: spec.preset_id,
            progress: 0,
        });
        let me = self.clone();
        tokio::spawn(async move { me.watch(generation, pid, spec).await });
        Ok(())
    }

    /// Arrête llama-server : SIGTERM au groupe, puis SIGKILL. Avec `wait_vram`,
    /// attend aussi que la VRAM soit rendue (changement de preset).
    pub async fn stop(&self, wait_vram: bool) {
        self.0.generation.fetch_add(1, Ordering::SeqCst);
        let Some(mut r) = self.0.running.lock().await.take() else {
            self.0.state.send_replace(EngineState::Stopped);
            return;
        };
        self.0.state.send_replace(EngineState::Stopping);
        signal_group(r.pid, libc::SIGTERM);
        if tokio::time::timeout(TERM_GRACE, r.child.wait()).await.is_err() {
            signal_group(r.pid, libc::SIGKILL);
            let _ = tokio::time::timeout(EXIT_WAIT, r.child.wait()).await;
        }
        // Le groupe entier (sous-processus éventuels).
        signal_group(r.pid, libc::SIGKILL);
        if wait_vram {
            self.wait_vram_settled().await;
        }
        self.log_line("[enginex] arrêté".into());
        self.0.state.send_replace(EngineState::Stopped);
    }

    async fn wait_vram_settled(&self) {
        let end = Instant::now() + VRAM_WAIT;
        let mut last: Option<Vec<i64>> = None;
        while Instant::now() < end {
            let used = match probe::read_gpus(&self.0.cfg.nvidia_smi).await {
                Ok(g) => g.iter().map(|g| g.used_mib).collect::<Vec<_>>(),
                Err(_) => return,
            };
            if last.as_ref() == Some(&used) {
                return;
            }
            last = Some(used);
            tokio::time::sleep(POLL).await;
        }
    }

    fn current(&self, generation: u64) -> bool {
        self.0.generation.load(Ordering::SeqCst) == generation
    }

    async fn watch(&self, generation: u64, pid: u32, spec: LaunchSpec) {
        let started = Instant::now();
        let health = format!("http://127.0.0.1:{}/health", self.0.cfg.port);
        let mut ready = false;
        loop {
            tokio::time::sleep(POLL).await;
            if !self.current(generation) {
                return;
            }
            let exited = {
                let mut g = self.0.running.lock().await;
                match g.as_mut() {
                    Some(r) if r.pid == pid => r.child.try_wait().ok().flatten().is_some(),
                    _ => return,
                }
            };
            if exited {
                // Laisse le temps aux dernières lignes d'arriver.
                tokio::time::sleep(Duration::from_millis(200)).await;
                if !self.current(generation) {
                    return;
                }
                self.0.running.lock().await.take();
                let last_lines = self.logs(20);
                self.0.state.send_replace(EngineState::Failed {
                    preset_id: spec.preset_id,
                    cause: failure_cause(&last_lines),
                    last_lines,
                });
                return;
            }
            if ready {
                continue;
            }
            let ok = self
                .0
                .http
                .get(&health)
                .send()
                .await
                .is_ok_and(|r| r.status().is_success());
            if ok {
                ready = true;
                self.log_line("[enginex] prêt".into());
                self.0.state.send_replace(EngineState::Ready {
                    preset_id: spec.preset_id,
                    pid,
                });
            } else if started.elapsed() > self.0.cfg.ready_timeout {
                self.stop(false).await;
                self.0.state.send_replace(EngineState::Failed {
                    preset_id: spec.preset_id,
                    cause: "délai de chargement dépassé".into(),
                    last_lines: self.logs(20),
                });
                return;
            } else {
                let p = load_progress(pid, spec.model_bytes);
                self.0.state.send_if_modified(|s| match s {
                    EngineState::Starting { progress, .. } if *progress != p => {
                        *progress = p;
                        true
                    }
                    _ => false,
                });
            }
        }
    }

    fn log_line(&self, line: String) {
        let mut l = self.0.logs.lock().expect("logs");
        if l.len() == LOG_LINES {
            l.pop_front();
        }
        l.push_back(line);
    }

    /// Lit une sortie ligne à ligne vers la mémoire et le fichier de log.
    fn pump(&self, r: impl tokio::io::AsyncRead + Unpin + Send + 'static) {
        let me = self.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(r).lines();
            let path = me.0.cfg.log_path.clone();
            let mut file = open_log(&path).await;
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(f) = file.as_mut() {
                    let _ = f.write_all(format!("{line}\n").as_bytes()).await;
                    if f.metadata().await.is_ok_and(|m| m.len() > LOG_MAX) {
                        let _ = tokio::fs::rename(&path, path.with_extension("log.1")).await;
                        file = open_log(&path).await;
                    }
                }
                me.log_line(line);
            }
        });
    }
}

async fn open_log(path: &Path) -> Option<tokio::fs::File> {
    if let Some(dir) = path.parent() {
        let _ = tokio::fs::create_dir_all(dir).await;
    }
    tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .ok()
}

fn signal_group(pid: u32, sig: libc::c_int) {
    // SAFETY: kill sur un groupe que nous avons créé (process_group(0) : pgid = pid).
    unsafe {
        libc::kill(-(pid as libc::pid_t), sig);
    }
}

/// max(octets lus, mémoire résidente) / taille du modèle, plafonné à 99 %.
fn load_progress(pid: u32, model_bytes: u64) -> u8 {
    if model_bytes == 0 {
        return 0;
    }
    let read = std::fs::read_to_string(format!("/proc/{pid}/io"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("read_bytes:"))
                .and_then(|v| v.trim().parse::<u64>().ok())
        })
        .unwrap_or(0);
    let rss = std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("VmRSS:"))
                .and_then(|v| v.trim().trim_end_matches("kB").trim().parse::<u64>().ok())
        })
        .map_or(0, |kb| kb * 1024);
    (read.max(rss) * 100 / model_bytes).min(99) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn causes() {
        let l = |s: &str| vec![s.to_string()];
        assert_eq!(
            failure_cause(&l("x: cudaMalloc failed: out of memory")),
            "VRAM insuffisante"
        );
        assert!(
            failure_cause(&l(
                "llama_model_load: error loading model: unknown model architecture: 'foo'"
            ))
            .starts_with("architecture")
        );
        assert_eq!(failure_cause(&l("rien")), "llama-server s'est arrêté");
    }
}
