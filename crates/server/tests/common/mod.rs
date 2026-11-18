//! Outils des tests du moteur : faux llama-server, faux Hugging Face,
//! faux registre OCI, appels de l'API avec session.
#![allow(dead_code)]

use std::{
    collections::HashMap,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Json, Router,
    body::Body,
    extract::{Path as UrlPath, State, connect_info::MockConnectInfo},
    http::{Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use enginex_server::{AppState, Config, app, db};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

pub const PASSWORD: &str = "correct horse battery";
pub const REPO: &str = "unsloth/Qwen3-0.6B-GGUF";
pub const BASE: &str = "Qwen/Qwen3-0.6B";

pub fn fake_bin() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.ancestors()
        .map(|d| d.join("fake-llama-server"))
        .find(|b| b.is_file())
        .unwrap_or_else(|| panic!("faux llama-server absent : lancer `cargo build -p enginex-fake-llama`"))
}

pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

pub fn sha_hex(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

/// GGUF v3 minimal d'un petit modèle qwen3.
pub fn fake_gguf() -> Vec<u8> {
    let mut kv: Vec<u8> = Vec::new();
    let mut n = 0u64;
    let mut key = |kv: &mut Vec<u8>, k: &str, t: u32| {
        kv.extend((k.len() as u64).to_le_bytes());
        kv.extend(k.as_bytes());
        kv.extend(t.to_le_bytes());
        n += 1;
    };
    key(&mut kv, "general.architecture", 8);
    kv.extend(5u64.to_le_bytes());
    kv.extend(b"qwen3");
    for (k, v) in [
        ("qwen3.block_count", 28u32),
        ("qwen3.context_length", 40960),
        ("qwen3.embedding_length", 1024),
        ("qwen3.attention.head_count", 16),
        ("qwen3.attention.head_count_kv", 8),
        ("qwen3.attention.key_length", 128),
        ("qwen3.attention.value_length", 128),
    ] {
        key(&mut kv, k, 4);
        kv.extend(v.to_le_bytes());
    }
    key(&mut kv, "tokenizer.chat_template", 8);
    kv.extend(4u64.to_le_bytes());
    kv.extend(b"{{x}}");
    let mut b = b"GGUF".to_vec();
    b.extend(3u32.to_le_bytes());
    b.extend(0u64.to_le_bytes());
    b.extend(n.to_le_bytes());
    b.extend(kv);
    b.extend([7u8; 4096]);
    b
}

fn card(name: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../hub/tests/cards")
        .join(name);
    std::fs::read_to_string(p).unwrap()
}

/// Faux Hugging Face : un dépôt GGUF (Q8_0) dont la base est Qwen/Qwen3-0.6B.
pub async fn serve_hf() -> String {
    let gguf = Arc::new(fake_gguf());
    let tree = json!([
        {"type": "file", "path": "README.md", "size": 10},
        {"type": "file", "path": "Qwen3-0.6B-Q8_0.gguf", "size": gguf.len(),
         "lfs": {"oid": sha_hex(&gguf), "size": gguf.len()}},
    ]);
    let g = gguf.clone();
    let app = Router::new()
        .route(
            "/api/models",
            get(|| async {
                Json(json!([{"id": REPO, "downloads": 10, "likes": 1, "lastModified": "2025-05-01", "gated": false}]))
            }),
        )
        .route(
            "/api/models/{owner}/{repo}",
            get(|UrlPath((o, r)): UrlPath<(String, String)>| async move {
                match format!("{o}/{r}").as_str() {
                    REPO => Json(json!({"id": REPO, "gated": false, "cardData": {"base_model": BASE}})).into_response(),
                    BASE => Json(json!({"id": BASE, "gated": false, "cardData": {}})).into_response(),
                    _ => StatusCode::NOT_FOUND.into_response(),
                }
            }),
        )
        .route(
            "/api/models/{owner}/{repo}/tree/main",
            get(move || async move { Json(tree.clone()) }),
        )
        .route(
            "/{owner}/{repo}/resolve/main/{*file}",
            get(move || {
                let g = g.clone();
                async move { (*g).clone() }
            }),
        )
        .route(
            "/{owner}/{repo}/raw/main/{*file}",
            get(|UrlPath((o, r, f)): UrlPath<(String, String, String)>| async move {
                match (format!("{o}/{r}").as_str(), f.as_str()) {
                    (REPO, "README.md") => card("unsloth-qwen3-0.6b-gguf.md").into_response(),
                    (BASE, "README.md") => card("qwen3-0.6b.md").into_response(),
                    (BASE, "generation_config.json") => {
                        Json(json!({"temperature": 0.6, "top_p": 0.95, "top_k": 20})).into_response()
                    }
                    _ => StatusCode::NOT_FOUND.into_response(),
                }
            }),
        );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}")
}

// ---- Faux registre OCI ----

#[derive(Default)]
pub struct Reg {
    manifests: HashMap<String, Value>,
    blobs: HashMap<String, Vec<u8>>,
}

pub type SharedReg = Arc<Mutex<Reg>>;

fn layer(entries: &[(&str, Vec<u8>, bool)]) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut b = tar::Builder::new(gz);
    for (name, data, link) in entries {
        let mut h = tar::Header::new_gnu();
        if *link {
            h.set_entry_type(tar::EntryType::Symlink);
            h.set_size(0);
            h.set_mode(0o777);
            b.append_link(&mut h, name, String::from_utf8(data.clone()).unwrap())
                .unwrap();
        } else {
            h.set_entry_type(tar::EntryType::Regular);
            h.set_size(data.len() as u64);
            h.set_mode(0o755);
            b.append_data(&mut h, name, &data[..]).unwrap();
        }
    }
    b.into_inner().unwrap().finish().unwrap()
}

/// Publie `server-cuda` = `build`, dont le llama-server lance le faux avec `env`.
pub fn publish(reg: &SharedReg, build: &str, env: &str) {
    let n = build.trim_start_matches('b');
    let script = format!("#!/bin/sh\nFAKE_BUILD={n} {env} exec {} \"$@\"\n", fake_bin().display()).into_bytes();
    let mut e = vec![("app/llama-server", script, false)];
    for l in [
        "libggml.so.0",
        "libggml-base.so.0",
        "libggml-cuda.so",
        "libmtmd.so.0",
        "libllama.so.0",
    ] {
        e.push((Box::leak(format!("app/{l}").into_boxed_str()), b"elf".to_vec(), false));
    }
    e.push(("app/libllama.so", b"libllama.so.0".to_vec(), true));
    let l = layer(&e);
    let d = format!("sha256:{}", sha_hex(&l));
    let image = json!({"schemaVersion": 2, "layers": [{"digest": d, "size": l.len()}]});
    let index = json!({
        "schemaVersion": 2,
        "manifests": [{"digest": format!("sha256:img-{build}"), "platform": {"os": "linux", "architecture": "amd64"}}],
        "annotations": {"org.opencontainers.image.version": build}
    });
    let mut r = reg.lock().unwrap();
    r.blobs.insert(d, l);
    r.manifests.insert("server-cuda".into(), index.clone());
    r.manifests.insert(format!("server-cuda-{build}"), index);
    r.manifests.insert(format!("sha256:img-{build}"), image);
}

async fn manifest(State(r): State<SharedReg>, UrlPath(reference): UrlPath<String>) -> Response {
    match r.lock().unwrap().manifests.get(&reference) {
        Some(m) => Json(m.clone()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn blob(State(r): State<SharedReg>, UrlPath(d): UrlPath<String>) -> Response {
    match r.lock().unwrap().blobs.get(&d) {
        Some(b) => b.clone().into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

pub async fn serve_registry() -> (String, SharedReg) {
    let reg: SharedReg = Arc::default();
    let app = Router::new()
        .route("/token", get(|| async { Json(json!({"token": "anon"})) }))
        .route("/v2/ggml-org/llama.cpp/manifests/{reference}", get(manifest))
        .route("/v2/ggml-org/llama.cpp/blobs/{digest}", get(blob))
        .with_state(reg.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (format!("http://{addr}"), reg)
}

// ---- Serveur EngineX ----

pub struct Env {
    pub state: AppState,
    pub router: Router,
    pub cookie: String,
    pub reg: SharedReg,
    pub dir: tempfile::TempDir,
}

pub struct Resp {
    pub status: StatusCode,
    pub body: Value,
    pub text: String,
    /// `nom=valeur` du Set-Cookie, s'il y en a un.
    pub cookie: Option<String>,
}

pub async fn call(r: &Router, method: &str, path: &str, body: Option<Value>, cookie: Option<&str>) -> Resp {
    let mut b = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "enginex:8090");
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    let req = match body {
        Some(v) => b
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(v.to_string())),
        None => b.body(Body::empty()),
    }
    .unwrap();
    let res = r.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let cookie = res
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .map(str::to_string);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Resp {
        status,
        body: serde_json::from_str(&text).unwrap_or(Value::Null),
        text,
        cookie,
    }
}

impl Env {
    pub async fn get(&self, path: &str) -> Resp {
        call(&self.router, "GET", path, None, Some(&self.cookie)).await
    }
    pub async fn send(&self, method: &str, path: &str, body: Value) -> Resp {
        call(&self.router, method, path, Some(body), Some(&self.cookie)).await
    }

    /// Relit `path` jusqu'à ce que `f` soit vrai (20 s au plus).
    pub async fn until(&self, path: &str, f: impl Fn(&Value) -> bool) -> Value {
        for _ in 0..200 {
            let r = self.get(path).await;
            if f(&r.body) {
                return r.body;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("délai dépassé sur {path} : {}", self.get(path).await.text);
    }
}

/// EngineX avec moteur de l'image = faux llama-server, HF et registre locaux,
/// tâches de fond lancées, session ouverte.
pub async fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let hf = serve_hf().await;
    let (registry, reg) = serve_registry().await;
    let config = Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        data_dir: dir.path().join("data"),
        llama_bin: None,
        image_bin: fake_bin(),
        nvidia_smi: "/nonexistent/nvidia-smi".into(),
        trust_proxy: false,
        models_dir: dir.path().join("models"),
        llama_port: free_port(),
        registry,
        hf_base: hf,
    };
    config.check_data_dir().unwrap();
    let pool = db::open(&config.data_dir).await.unwrap();
    let state = AppState::new(config, pool);
    enginex_server::auth::ensure_setup_code(&state).await.unwrap();
    state.start_background().await.unwrap();
    let router = app(state.clone()).layer(MockConnectInfo(SocketAddr::from(([10, 0, 0, 1], 1234))));
    let code = state.setup_code().unwrap();
    let res = call(
        &router,
        "POST",
        "/api/auth/setup",
        Some(json!({"code": code, "password": PASSWORD})),
        None,
    )
    .await;
    assert_eq!(res.status, StatusCode::OK, "{}", res.text);
    Env {
        state,
        router,
        cookie: res.cookie.expect("Set-Cookie"),
        reg,
        dir,
    }
}
