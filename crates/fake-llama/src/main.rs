//! Faux llama-server pour les tests (aucun GPU). Imite les sorties utilisées
//! par EngineX ; réglable par variables d'environnement :
//! - `FAKE_BUILD` (défaut 12464) : numéro écrit par `--version` ;
//! - `FAKE_GPUS` (défaut 2) : cartes listées par `--list-devices` ;
//! - `FAKE_NO_FIT=1` : `--help` sans `--fit` ;
//! - `FAKE_LOAD_MS` (défaut 300) : `/health` répond 503 pendant ce temps ;
//! - `FAKE_CRASH_MS` : sort avec le code 1 après ce délai ;
//! - `FAKE_OOM=1` : écrit une erreur cudaMalloc et sort.

use std::time::{Duration, Instant};

use axum::{Json, Router, http::StatusCode, routing::get, routing::post};

fn env_u64(k: &str, d: u64) -> u64 {
    std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}

fn arg<'a>(args: &'a [String], names: &[&str]) -> Option<&'a str> {
    args.iter()
        .position(|a| names.contains(&a.as_str()))
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") => {
            eprintln!("load_backend: loaded CUDA backend");
            eprintln!("version: {} (d3c3e242)", env_u64("FAKE_BUILD", 12464));
            eprintln!("built with cc for x86_64-linux-gnu");
            return;
        }
        Some("--list-devices") => {
            eprintln!("Available devices:");
            for i in 0..env_u64("FAKE_GPUS", 2) {
                eprintln!("  CUDA{i}: NVIDIA Fake {i} (16311 MiB, 15900 MiB free)");
            }
            return;
        }
        Some("--help") => {
            println!("-m, --model FNAME\n-c, --ctx-size N\n-ngl N\n-ts, --tensor-split N0,N1");
            if std::env::var("FAKE_NO_FIT").as_deref() != Ok("1") {
                println!("-fit, --fit [on|off]   whether to adjust unset arguments\n-fitt, --fit-target MiB0,MiB1,...");
            }
            return;
        }
        _ => {}
    }
    eprintln!("main: argv = {}", args.join(" "));
    if std::env::var("FAKE_OOM").as_deref() == Ok("1") {
        eprintln!(
            "ggml_backend_cuda_buffer_type_alloc_buffer: allocating 9000.00 MiB on device 0: cudaMalloc failed: out of memory"
        );
        eprintln!("llama_model_load: error loading model");
        std::process::exit(1);
    }
    let port: u16 = arg(&args, &["--port"]).and_then(|p| p.parse().ok()).unwrap_or(8080);
    let n_ctx: u64 = arg(&args, &["-c", "--ctx-size"])
        .and_then(|p| p.parse().ok())
        .unwrap_or(4096);
    if let Ok(ms) = std::env::var("FAKE_CRASH_MS").map(|v| v.parse::<u64>().unwrap_or(0)) {
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            eprintln!("Segmentation fault");
            std::process::exit(1);
        });
    }
    let started = Instant::now();
    let load = Duration::from_millis(env_u64("FAKE_LOAD_MS", 300));
    let app = Router::new()
        .route(
            "/health",
            get(move || async move {
                if started.elapsed() < load {
                    (StatusCode::SERVICE_UNAVAILABLE, "loading")
                } else {
                    (StatusCode::OK, "ok")
                }
            }),
        )
        .route(
            "/props",
            get(move || async move {
                Json(serde_json::json!({
                    "build_info": "b12464-d3c3e242",
                    "chat_template": "{{ tools }}",
                    "default_generation_settings": { "n_ctx": n_ctx },
                }))
            }),
        )
        .route(
            "/apply-template",
            post(
                |Json(v): Json<serde_json::Value>| async move { Json(serde_json::json!({ "prompt": v.to_string() })) },
            ),
        );
    let l = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.expect("port");
    eprintln!("main: server is listening on http://127.0.0.1:{port}");
    axum::serve(l, app).await.expect("serve");
}
