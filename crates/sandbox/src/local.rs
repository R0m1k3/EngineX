//! Backend local : `codex app-server` lancé en processus (aucune isolation,
//! développement et tests). Chaque sandbox laisse une fiche
//! `<state_dir>/<session>.json` (pid, propriétaire, preset) : un EngineX
//! relancé retrouve et arrête les processus d'un arrêt brutal.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Mutex,
};

use serde_json::{Value, json};

use crate::{HELPER_STDOUT_MAX, Handle, HelperOutcome, HelperSpec, OUTPUT_TOO_LARGE, Owned, Spec};

pub struct Local {
    bin: PathBuf,
    state_dir: PathBuf,
    children: Mutex<HashMap<String, tokio::process::Child>>,
}

fn free_port() -> anyhow::Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

fn alive(pid: i32) -> bool {
    // SAFETY: kill(pid, 0) ne fait que tester l'existence du processus.
    pid > 0 && unsafe { libc::kill(pid, 0) } == 0
}

fn kill_group(pid: i32) {
    if pid > 0 {
        // SAFETY: signal envoyé au groupe créé pour ce processus (ou au
        // processus seul s'il n'a pas de groupe).
        unsafe {
            if libc::kill(-pid, libc::SIGKILL) != 0 {
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
}

impl Local {
    pub fn new(bin: PathBuf, state_dir: PathBuf) -> Self {
        Self {
            bin,
            state_dir,
            children: Mutex::new(HashMap::new()),
        }
    }

    fn card(&self, session_id: &str) -> PathBuf {
        self.state_dir.join(format!("{session_id}.json"))
    }

    fn log_path(&self, session_id: &str) -> PathBuf {
        self.state_dir.join(format!("{session_id}.log"))
    }

    pub async fn start(&self, spec: &Spec) -> anyhow::Result<Handle> {
        std::fs::create_dir_all(&self.state_dir)?;
        let port = free_port()?;
        let log = std::fs::File::create(self.log_path(&spec.session_id))?;
        let m = &spec.mounts;
        let mut cmd = tokio::process::Command::new(&self.bin);
        cmd.arg("app-server")
            .arg("--listen")
            .arg(format!("ws://127.0.0.1:{port}"))
            .args(["--ws-auth", "capability-token", "--ws-token-file"])
            .arg(m.run.join("token"))
            .current_dir(&m.topic)
            .env("CODEX_HOME", &m.codex_home)
            .envs(spec.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .process_group(0)
            .kill_on_drop(true);
        let child = cmd
            .spawn()
            .map_err(|e| anyhow::anyhow!("lancement de {} : {e}", self.bin.display()))?;
        let pid = child.id().unwrap_or(0) as i32;
        std::fs::write(
            self.card(&spec.session_id),
            json!({"pid": pid, "owner": spec.owner, "preset": spec.preset}).to_string(),
        )?;
        self.children
            .lock()
            .expect("verrou")
            .insert(spec.session_id.clone(), child);
        Ok(Handle {
            ws_url: format!("ws://127.0.0.1:{port}"),
            readyz_url: format!("http://127.0.0.1:{port}/readyz"),
            cwd: m.topic.display().to_string(),
            attachments: m.attachments.display().to_string(),
        })
    }

    pub async fn stop(&self, session_id: &str) -> anyhow::Result<()> {
        let child = self.children.lock().expect("verrou").remove(session_id);
        let card = self.card(session_id);
        let pid = match &child {
            Some(c) => c.id().map(|p| p as i32),
            None => read_card(&card).and_then(|v| v["pid"].as_i64()).map(|p| p as i32),
        };
        if let Some(pid) = pid {
            kill_group(pid);
        }
        if let Some(mut c) = child {
            let _ = c.kill().await;
        }
        let _ = std::fs::remove_file(card);
        Ok(())
    }

    pub async fn list_owned(&self, owner: &str) -> anyhow::Result<Vec<Owned>> {
        let mut out = Vec::new();
        let Ok(dir) = std::fs::read_dir(&self.state_dir) else {
            return Ok(out);
        };
        for e in dir.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let Some(v) = read_card(&p) else { continue };
            let pid = v["pid"].as_i64().unwrap_or(0) as i32;
            if v["owner"] != owner {
                continue;
            }
            let session_id = p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string();
            if !alive(pid) {
                let _ = std::fs::remove_file(&p);
                continue;
            }
            out.push(Owned {
                session_id,
                preset: v["preset"].as_str().unwrap_or("").to_string(),
            });
        }
        Ok(out)
    }

    /// Script du conteneur d'aide en processus hôte (sans isolation).
    pub async fn run_helper(&self, spec: &HelperSpec, timeout: std::time::Duration) -> anyhow::Result<HelperOutcome> {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c")
            .arg(spec.script)
            .current_dir("/")
            .envs(spec.env.iter().map(|(k, v)| (k, v)))
            .envs(spec.binds.iter().map(|(p, var, _)| (*var, p.as_os_str())))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let child = cmd
            .spawn()
            .map_err(|e| anyhow::anyhow!("lancement du script d'aide : {e}"))?;
        let out = tokio::time::timeout(timeout, child.wait_with_output())
            .await
            .map_err(|_| anyhow::anyhow!("script d'aide : délai de {} s dépassé", timeout.as_secs()))??;
        if out.stdout.len() > HELPER_STDOUT_MAX {
            anyhow::bail!(OUTPUT_TOO_LARGE);
        }
        Ok(HelperOutcome {
            code: out.status.code().map(i64::from).unwrap_or(-1),
            stdout: out.stdout,
            stderr: crate::tail(&String::from_utf8_lossy(&out.stderr), 200),
        })
    }

    pub async fn logs(&self, session_id: &str) -> String {
        let s = std::fs::read_to_string(self.log_path(session_id)).unwrap_or_default();
        crate::tail(&s, 30)
    }
}

fn read_card(p: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(p).ok()?).ok()
}
