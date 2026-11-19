//! Superviseur avec le faux llama-server.

mod common;

use std::time::Duration;

use enginex_engine::supervisor::{EngineState, LaunchSpec, Supervisor, SupervisorConfig};

fn sup(dir: &std::path::Path, port: u16, timeout: Duration) -> Supervisor {
    Supervisor::new(SupervisorConfig {
        port,
        log_path: dir.join("logs/llama-server.log"),
        nvidia_smi: "/nonexistent".into(),
        ready_timeout: timeout,
    })
}

fn spec(bin: std::path::PathBuf, port: u16) -> LaunchSpec {
    LaunchSpec {
        bin,
        argv: vec![
            "-m".into(),
            "m.gguf".into(),
            "--port".into(),
            port.to_string(),
            "-c".into(),
            "4096".into(),
        ],
        model_bytes: 1 << 30,
        preset_id: 7,
        env: Vec::new(),
        fallback_argv: None,
    }
}

async fn until(s: &Supervisor, f: impl Fn(&EngineState) -> bool) -> EngineState {
    let mut rx = s.subscribe();
    let r = tokio::time::timeout(Duration::from_secs(10), rx.wait_for(|st| f(st))).await;
    r.expect("délai").expect("canal").clone()
}

#[tokio::test]
async fn starts_becomes_ready_and_stops() {
    let dir = tempfile::tempdir().unwrap();
    let port = common::free_port();
    let s = sup(dir.path(), port, Duration::from_secs(10));
    s.start(spec(common::fake_bin(), port)).await.unwrap();
    assert!(matches!(s.state(), EngineState::Starting { preset_id: 7, .. }));
    let st = until(&s, |st| matches!(st, EngineState::Ready { .. })).await;
    let EngineState::Ready { pid, .. } = st else {
        unreachable!()
    };
    assert!(s.logs(50).iter().any(|l| l.contains("server is listening")));
    assert!(
        std::fs::read_to_string(dir.path().join("logs/llama-server.log"))
            .unwrap()
            .contains("argv")
    );
    let client = reqwest::Client::new();
    let props = enginex_engine::probe::props(&client, port).await.unwrap();
    assert_eq!(
        props
            .pointer("/default_generation_settings/n_ctx")
            .and_then(|v| v.as_u64()),
        Some(4096)
    );
    assert!(enginex_engine::probe::tools_probe(&client, port).await.unwrap());
    let t = std::time::Instant::now();
    s.stop(false).await;
    assert!(t.elapsed() < Duration::from_secs(15));
    assert_eq!(s.state(), EngineState::Stopped);
    // Le processus n'existe plus.
    assert!(
        !std::path::Path::new(&format!("/proc/{pid}/status")).exists() || {
            std::fs::read_to_string(format!("/proc/{pid}/status"))
                .unwrap()
                .contains("zombie")
        }
    );
}

#[tokio::test]
async fn oom_gives_cause_without_restart() {
    let dir = tempfile::tempdir().unwrap();
    let port = common::free_port();
    let s = sup(dir.path(), port, Duration::from_secs(10));
    let bin = common::wrapper(&dir.path().join("oom"), "FAKE_OOM=1");
    s.start(spec(bin, port)).await.unwrap();
    let st = until(&s, |st| matches!(st, EngineState::Failed { .. })).await;
    let EngineState::Failed {
        cause,
        last_lines,
        preset_id,
    } = st
    else {
        unreachable!()
    };
    assert_eq!((cause.as_str(), preset_id), ("VRAM insuffisante", 7));
    assert!(last_lines.iter().any(|l| l.contains("cudaMalloc")));
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(matches!(s.state(), EngineState::Failed { .. }), "pas de relance");
}

#[tokio::test]
async fn crash_after_ready() {
    let dir = tempfile::tempdir().unwrap();
    let port = common::free_port();
    let s = sup(dir.path(), port, Duration::from_secs(10));
    let bin = common::wrapper(&dir.path().join("crash"), "FAKE_LOAD_MS=0 FAKE_CRASH_MS=1500");
    s.start(spec(bin, port)).await.unwrap();
    until(&s, |st| matches!(st, EngineState::Ready { .. })).await;
    let st = until(&s, |st| matches!(st, EngineState::Failed { .. })).await;
    let EngineState::Failed { cause, .. } = st else {
        unreachable!()
    };
    assert!(cause.contains("segmentation"), "{cause}");
}

#[tokio::test]
async fn load_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let port = common::free_port();
    let s = sup(dir.path(), port, Duration::from_millis(800));
    let bin = common::wrapper(&dir.path().join("slow"), "FAKE_LOAD_MS=60000");
    s.start(spec(bin, port)).await.unwrap();
    let st = until(&s, |st| matches!(st, EngineState::Failed { .. })).await;
    let EngineState::Failed { cause, .. } = st else {
        unreachable!()
    };
    assert_eq!(cause, "délai de chargement dépassé");
}

#[tokio::test]
async fn restart_replaces_previous() {
    let dir = tempfile::tempdir().unwrap();
    let port = common::free_port();
    let s = sup(dir.path(), port, Duration::from_secs(10));
    s.start(spec(common::fake_bin(), port)).await.unwrap();
    until(&s, |st| matches!(st, EngineState::Ready { .. })).await;
    let mut sp = spec(common::fake_bin(), port);
    sp.preset_id = 8;
    s.start(sp).await.unwrap();
    let st = until(&s, |st| matches!(st, EngineState::Ready { preset_id: 8, .. })).await;
    assert_eq!(st.preset_id(), Some(8));
    s.stop(false).await;
}
