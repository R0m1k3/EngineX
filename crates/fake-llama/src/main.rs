//! Faux llama-server pour les tests (aucun GPU). Imite les sorties utilisées
//! par EngineX ; réglable par variables d'environnement :
//! - `FAKE_BUILD` (défaut 12464) : numéro écrit par `--version` ;
//! - `FAKE_GPUS` (défaut 2) : cartes listées par `--list-devices` ;
//! - `FAKE_NO_FIT=1` : `--help` sans `--fit` ;
//! - `FAKE_NO_ACCEL=1` : `--help` sans les options MTP / MoE ;
//! - `FAKE_SPEC_FAIL=1` : s'arrête au chargement si `--spec-type` est passé ;
//!   `soft` : le signale et continue sans (brouillon refusé) ;
//! - `FAKE_FIT_CTX=N` : avec `--fit on`, contexte ramené à N (VRAM courte) ;
//! - `FAKE_LOAD_MS` (défaut 300) : `/health` répond 503 pendant ce temps ;
//! - `FAKE_CRASH_MS` : sort avec le code 1 après ce délai ;
//! - `FAKE_OOM=1` : écrit une erreur cudaMalloc et sort ;
//! - `FAKE_NO_TOOLS=1` : `/apply-template` rend un texte sans les outils
//!   (la sonde « outils » répond `false`) ;
//! - `FAKE_CHAT_SCRIPT` : fichier JSON, liste de tours rendus par
//!   `/v1/chat/completions` (le tour n répond à la n-ième requête ; au-delà, le
//!   dernier tour) ; un tour = `{reasoning?, content?, tool_calls?:[{name,
//!   arguments, id?}], finish_reason?, error?, no_done?}` ; sans script : « ok » ;
//! - `FAKE_CHAT_LOG` : fichier où chaque corps reçu est ajouté (une ligne JSON) ;
//! - `FAKE_CTX_LIMIT` : corps plus long (en caractères) → 400
//!   `exceed_context_size_error` ;
//! - `FAKE_CHUNK_DELAY_MS` (défaut 0) : pause entre deux morceaux du flux.
//!
//! Sous-commandes :
//! - `capture --port P --log F` : reçoit `POST /v1/responses`, ajoute le corps
//!   brut à F (une ligne par requête) et répond un flux Responses minimal ;
//! - `mcp` : serveur MCP minimal sur stdio (outil `echo(text)`) ;
//! - `app-server --listen ws://… --ws-auth capability-token --ws-token-file F` :
//!   faux `codex app-server` scripté (voir `appserver.rs`).

mod appserver;

use std::io::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::{
    Json, Router,
    body::{Body, Bytes},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

fn env_u64(k: &str, d: u64) -> u64 {
    std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}

fn arg<'a>(args: &'a [String], names: &[&str]) -> Option<&'a str> {
    args.iter()
        .position(|a| names.contains(&a.as_str()))
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

fn append_line(path: &str, line: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") => {
            eprintln!("load_backend: loaded CUDA backend");
            // Forme de b12464 (binaire officiel).
            eprintln!(
                "version: 1.0.0-dev (build {}, commit d3c3e242c)",
                env_u64("FAKE_BUILD", 12464)
            );
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
            if std::env::var("FAKE_NO_ACCEL").as_deref() != Ok("1") {
                println!(
                    "-cmoe, --cpu-moe   keep all MoE weights in the CPU\n-ncmoe, --n-cpu-moe N\n\
                     --spec-type none,draft-simple,draft-mtp,ngram-mod\n--spec-draft-n-max N"
                );
            }
            if std::env::var("FAKE_NO_FIT").as_deref() != Ok("1") {
                println!("-fit, --fit [on|off]   whether to adjust unset arguments\n-fitt, --fit-target MiB0,MiB1,...");
            }
            return;
        }
        Some("capture") => return capture(&args).await,
        Some("mcp") => return mcp().await,
        Some("app-server") => return appserver::run(&args).await,
        _ => {}
    }
    eprintln!("main: argv = {}", args.join(" "));
    let _ = SPEC.set(args.iter().any(|a| a == "--spec-type"));
    if *SPEC.get().unwrap_or(&false) && std::env::var("FAKE_SPEC_FAIL").as_deref() == Ok("1") {
        eprintln!("common_speculative_init: failed to create mtp context");
        eprintln!("srv load_model: failed to initialize speculative decoding context");
        std::process::exit(1);
    }
    if *SPEC.get().unwrap_or(&false) && std::env::var("FAKE_SPEC_FAIL").as_deref() == Ok("soft") {
        // Comme b12464 avec un brouillon au vocabulaire différent : continue sans.
        eprintln!(
            "srv load_model: failed to initialize speculative decoding context: draft model vocab type must match"
        );
    }
    if let Ok(v) = std::env::var("CUDA_SCALE_LAUNCH_QUEUES") {
        eprintln!("env: CUDA_SCALE_LAUNCH_QUEUES={v}");
    }
    if std::env::var("FAKE_OOM").as_deref() == Ok("1") {
        eprintln!(
            "ggml_backend_cuda_buffer_type_alloc_buffer: allocating 9000.00 MiB on device 0: cudaMalloc failed: out of memory"
        );
        eprintln!("llama_model_load: error loading model");
        std::process::exit(1);
    }
    let port: u16 = arg(&args, &["--port"]).and_then(|p| p.parse().ok()).unwrap_or(8080);
    let mut n_ctx: u64 = arg(&args, &["-c", "--ctx-size"])
        .and_then(|p| p.parse().ok())
        .unwrap_or(4096);
    // `--fit on` qui réduit le contexte faute de VRAM.
    if let (Some("on"), Ok(max)) = (
        arg(&args, &["--fit"]),
        std::env::var("FAKE_FIT_CTX").map(|v| v.parse::<u64>().unwrap_or(u64::MAX)),
    ) && max < n_ctx
    {
        eprintln!("llama_params_fit_impl: context size reduced from {n_ctx} to {max}");
        n_ctx = max;
    }
    if let Ok(ms) = std::env::var("FAKE_CRASH_MS").map(|v| v.parse::<u64>().unwrap_or(0)) {
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            eprintln!("Segmentation fault");
            std::process::exit(1);
        });
    }
    let started = Instant::now();
    let load = Duration::from_millis(env_u64("FAKE_LOAD_MS", 300));
    let no_tools = std::env::var("FAKE_NO_TOOLS").as_deref() == Ok("1");
    let chat = Arc::new(Chat::from_env());
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
                Json(json!({
                    "build_info": "b12464-d3c3e242",
                    "chat_template": "{{ tools }}",
                    "default_generation_settings": { "n_ctx": n_ctx },
                }))
            }),
        )
        .route(
            "/apply-template",
            post(move |Json(v): Json<Value>| async move {
                let prompt = if no_tools {
                    "<|user|>hello".to_string()
                } else {
                    v.to_string()
                };
                Json(json!({ "prompt": prompt }))
            }),
        )
        .route(
            "/v1/chat/completions",
            post(move |body: Bytes| {
                let chat = chat.clone();
                async move { chat.handle(body) }
            }),
        );
    let l = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.expect("port");
    eprintln!("main: server is listening on http://127.0.0.1:{port}");
    axum::serve(l, app).await.expect("serve");
}

// ---------------------------------------------------------------------------
// /v1/chat/completions scripté

#[derive(Debug, Clone, Default, Deserialize)]
struct Turn {
    reasoning: Option<String>,
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCall>,
    finish_reason: Option<String>,
    error: Option<String>,
    #[serde(default)]
    no_done: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct ToolCall {
    name: String,
    arguments: String,
    id: Option<String>,
}

struct Chat {
    script: Vec<Turn>,
    n: AtomicUsize,
    log: Option<String>,
    ctx_limit: Option<usize>,
    delay: Duration,
}

impl Chat {
    fn from_env() -> Chat {
        let script = std::env::var("FAKE_CHAT_SCRIPT")
            .ok()
            .map(|p| {
                let raw = std::fs::read_to_string(&p).expect("FAKE_CHAT_SCRIPT illisible");
                serde_json::from_str(&raw).expect("FAKE_CHAT_SCRIPT invalide")
            })
            .unwrap_or_default();
        Chat {
            script,
            n: AtomicUsize::new(0),
            log: std::env::var("FAKE_CHAT_LOG").ok(),
            ctx_limit: std::env::var("FAKE_CTX_LIMIT").ok().and_then(|v| v.parse().ok()),
            delay: Duration::from_millis(env_u64("FAKE_CHUNK_DELAY_MS", 0)),
        }
    }

    fn turn(&self, i: usize) -> Turn {
        if self.script.is_empty() {
            return Turn {
                content: Some("ok".into()),
                ..Turn::default()
            };
        }
        self.script[i.min(self.script.len() - 1)].clone()
    }

    fn handle(&self, body: Bytes) -> Response {
        let text = String::from_utf8_lossy(&body).to_string();
        if let Some(log) = &self.log {
            append_line(log, text.trim());
        }
        let req: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => return err(StatusCode::BAD_REQUEST, "invalid_request_error", &e.to_string()),
        };
        if let Some(limit) = self.ctx_limit
            && text.chars().count() > limit
        {
            let mut e = json!({"code": 400, "type": "exceed_context_size_error",
                "message": "the request exceeds the available context size, try increasing it"});
            e["n_prompt_tokens"] = json!(text.len() / 4);
            e["n_ctx"] = json!(limit / 4);
            return (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response();
        }
        let turn = self.turn(self.n.fetch_add(1, Ordering::SeqCst));
        let model = req.get("model").and_then(Value::as_str).unwrap_or("fake").to_string();
        if req.get("stream").and_then(Value::as_bool) != Some(true) {
            return Json(full_completion(&turn, &model)).into_response();
        }
        let usage = req.pointer("/stream_options/include_usage").and_then(Value::as_bool) == Some(true);
        let (tx, rx) = mpsc::channel::<Result<Bytes, std::io::Error>>(16);
        let delay = self.delay;
        tokio::spawn(async move {
            for chunk in stream_chunks(&turn, &model, usage) {
                if !delay.is_zero() {
                    tokio::time::sleep(delay).await;
                }
                if tx.send(Ok(Bytes::from(chunk))).await.is_err() {
                    return;
                }
            }
        });
        Response::builder()
            .header(header::CONTENT_TYPE, "text/event-stream")
            .body(Body::from_stream(ReceiverStream::new(rx)))
            .expect("réponse")
    }
}

fn err(status: StatusCode, kind: &str, msg: &str) -> Response {
    (
        status,
        Json(json!({"error": {"code": status.as_u16(), "type": kind, "message": msg}})),
    )
        .into_response()
}

fn pieces(s: &str) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    chars.chunks(4).map(|c| c.iter().collect()).collect()
}

fn finish_of(turn: &Turn) -> String {
    turn.finish_reason.clone().unwrap_or_else(|| {
        if turn.tool_calls.is_empty() {
            "stop"
        } else {
            "tool_calls"
        }
        .into()
    })
}

fn call_id(i: usize, tc: &ToolCall) -> String {
    tc.id.clone().unwrap_or_else(|| format!("call_fake{i}"))
}

/// `--spec-type` reçu : les `timings` portent alors `draft_n*`.
static SPEC: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// `timings` de llama-server (dernier morceau du flux).
fn timings_json() -> Value {
    let mut t = json!({"prompt_n": 60, "prompt_ms": 120.0, "prompt_per_second": 500.0,
        "predicted_n": 20, "predicted_ms": 400.0, "predicted_per_second": 50.0});
    if SPEC.get().copied().unwrap_or(false) {
        t["draft_n"] = json!(16);
        t["draft_n_accepted"] = json!(12);
    }
    t
}

fn usage_json() -> Value {
    json!({"prompt_tokens": 100, "completion_tokens": 20, "total_tokens": 120,
        "prompt_tokens_details": {"cached_tokens": 40}})
}

fn full_completion(turn: &Turn, model: &str) -> Value {
    let mut msg = json!({"role": "assistant", "content": turn.content.clone().unwrap_or_default()});
    if let Some(r) = &turn.reasoning {
        msg["reasoning_content"] = json!(r);
    }
    if !turn.tool_calls.is_empty() {
        msg["tool_calls"] = turn
            .tool_calls
            .iter()
            .enumerate()
            .map(|(i, tc)| {
                json!({"id": call_id(i, tc), "type": "function",
                    "function": {"name": tc.name, "arguments": tc.arguments}})
            })
            .collect();
    }
    json!({"id": "chatcmpl-fake", "object": "chat.completion", "created": 0, "model": model,
        "choices": [{"index": 0, "message": msg, "finish_reason": finish_of(turn)}],
        "usage": usage_json()})
}

fn stream_chunks(turn: &Turn, model: &str, usage: bool) -> Vec<String> {
    let chunk = |delta: Value, finish: Value| {
        let v = json!({"id": "chatcmpl-fake", "object": "chat.completion.chunk", "created": 0,
            "model": model, "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]});
        format!("data: {v}\n\n")
    };
    let mut out = vec![chunk(json!({"role": "assistant", "content": null}), Value::Null)];
    if let Some(r) = &turn.reasoning {
        for p in pieces(r) {
            out.push(chunk(json!({"reasoning_content": p}), Value::Null));
        }
    }
    if let Some(c) = &turn.content {
        for p in pieces(c) {
            out.push(chunk(json!({"content": p}), Value::Null));
        }
    }
    for (i, tc) in turn.tool_calls.iter().enumerate() {
        out.push(chunk(
            json!({"tool_calls": [{"index": i, "id": call_id(i, tc), "type": "function",
                "function": {"name": tc.name, "arguments": ""}}]}),
            Value::Null,
        ));
        for p in pieces(&tc.arguments) {
            out.push(chunk(
                json!({"tool_calls": [{"index": i, "function": {"arguments": p}}]}),
                Value::Null,
            ));
        }
    }
    if let Some(e) = &turn.error {
        let v = json!({"error": {"code": 500, "type": "server_error", "message": e}});
        out.push(format!("data: {v}\n\n"));
        return out;
    }
    if turn.no_done {
        return out;
    }
    out.push(chunk(json!({}), json!(finish_of(turn))));
    if usage {
        let v = json!({"id": "chatcmpl-fake", "object": "chat.completion.chunk", "created": 0,
            "model": model, "choices": [], "usage": usage_json(), "timings": timings_json()});
        out.push(format!("data: {v}\n\n"));
    }
    out.push("data: [DONE]\n\n".into());
    out
}

// ---------------------------------------------------------------------------
// capture : relève la forme réelle des requêtes Responses de Codex

async fn capture(args: &[String]) {
    let port: u16 = arg(args, &["--port"]).and_then(|p| p.parse().ok()).expect("--port");
    let log = arg(args, &["--log"]).expect("--log").to_string();
    let n = Arc::new(AtomicUsize::new(0));
    let app = Router::new().route(
        "/v1/responses",
        post(move |body: Bytes| {
            let log = log.clone();
            let n = n.clone();
            async move {
                let raw = String::from_utf8_lossy(&body).to_string();
                append_line(&log, raw.trim());
                let i = n.fetch_add(1, Ordering::SeqCst);
                let id = format!("resp_capture{i}");
                let item = json!({"type": "message", "id": format!("msg_capture{i}"), "role": "assistant",
                    "status": "completed", "content": [{"type": "output_text", "text": "ok", "annotations": []}]});
                let events = [
                    (
                        "response.created",
                        json!({"type": "response.created", "sequence_number": 0,
                        "response": {"id": id, "object": "response", "status": "in_progress", "output": []}}),
                    ),
                    (
                        "response.output_item.added",
                        json!({"type": "response.output_item.added",
                        "sequence_number": 1, "output_index": 0, "item": item}),
                    ),
                    (
                        "response.output_text.delta",
                        json!({"type": "response.output_text.delta",
                        "sequence_number": 2, "output_index": 0, "content_index": 0,
                        "item_id": format!("msg_capture{i}"), "delta": "ok"}),
                    ),
                    (
                        "response.output_item.done",
                        json!({"type": "response.output_item.done",
                        "sequence_number": 3, "output_index": 0, "item": item}),
                    ),
                    (
                        "response.completed",
                        json!({"type": "response.completed", "sequence_number": 4,
                        "response": {"id": id, "object": "response", "status": "completed", "output": [item],
                        "usage": {"input_tokens": 10, "input_tokens_details": {"cached_tokens": 0},
                            "output_tokens": 1, "output_tokens_details": {"reasoning_tokens": 0},
                            "total_tokens": 11}}}),
                    ),
                ];
                let sse: String = events
                    .iter()
                    .map(|(n, d)| format!("event: {n}\ndata: {d}\n\n"))
                    .collect();
                ([(header::CONTENT_TYPE, "text/event-stream")], sse)
            }
        }),
    );
    let l = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.expect("port");
    eprintln!("capture: listening on http://127.0.0.1:{port}");
    axum::serve(l, app).await.expect("serve");
}

// ---------------------------------------------------------------------------
// mcp : serveur MCP minimal (JSON-RPC, une ligne par message sur stdio)

async fn mcp() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut out = tokio::io::stdout();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(id) = msg.get("id").cloned() else { continue }; // notification
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let reply = match method {
            "initialize" => {
                let version = msg
                    .pointer("/params/protocolVersion")
                    .cloned()
                    .unwrap_or(json!("2025-06-18"));
                json!({"jsonrpc": "2.0", "id": id, "result": {"protocolVersion": version,
                    "capabilities": {"tools": {}}, "serverInfo": {"name": "test", "version": "0.1.0"}}})
            }
            "tools/list" => json!({"jsonrpc": "2.0", "id": id, "result": {"tools": [{
                "name": "echo", "description": "Renvoie le texte reçu.",
                "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}},
                    "required": ["text"]}}]}}),
            "tools/call" => {
                let text = msg
                    .pointer("/params/arguments/text")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                json!({"jsonrpc": "2.0", "id": id, "result": {
                    "content": [{"type": "text", "text": format!("echo: {text}")}], "isError": false}})
            }
            "ping" => json!({"jsonrpc": "2.0", "id": id, "result": {}}),
            _ => json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not found"}}),
        };
        let mut s = reply.to_string();
        s.push('\n');
        if out.write_all(s.as_bytes()).await.is_err() {
            break;
        }
        let _ = out.flush().await;
    }
}
