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
use enginex_server::{
    AppState, Config, app,
    config::{AutoUpdateTimings, SandboxConfig, SandboxTimings, Timings},
    db,
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

pub const PASSWORD: &str = "correct horse battery";
pub const REPO: &str = "unsloth/Qwen3-0.6B-GGUF";
pub const BASE: &str = "Qwen/Qwen3-0.6B";
/// Dépôt de test avec un projecteur de vision.
pub const VISION: &str = "test/vision";
/// Dépôt d'un MoE avec couches MTP (`nextn_predict_layers`).
pub const MTP: &str = "test/mtp-moe";

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
    fake_gguf_with(&[])
}

/// Le même, avec des clés `qwen3.*` entières en plus.
pub fn fake_gguf_with(extra: &[(&str, u32)]) -> Vec<u8> {
    fake_gguf_tensors(extra, &[])
}

/// Le même, avec une table de tenseurs (noms seuls, poids factices).
pub fn fake_gguf_tensors(extra: &[(&str, u32)], tensors: &[&str]) -> Vec<u8> {
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
    ]
    .into_iter()
    .chain(extra.iter().copied())
    {
        key(&mut kv, k, 4);
        kv.extend(v.to_le_bytes());
    }
    key(&mut kv, "tokenizer.chat_template", 8);
    kv.extend(5u64.to_le_bytes());
    kv.extend(b"{{x}}");
    let mut b = b"GGUF".to_vec();
    b.extend(3u32.to_le_bytes());
    b.extend((tensors.len() as u64).to_le_bytes());
    b.extend(n.to_le_bytes());
    b.extend(kv);
    for t in tensors {
        b.extend((t.len() as u64).to_le_bytes());
        b.extend(t.as_bytes());
        b.extend(1u32.to_le_bytes());
        b.extend(16u64.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        b.extend(0u64.to_le_bytes());
    }
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
    let vision_tree = json!([
        {"type": "file", "path": "vision-Q8_0.gguf", "size": gguf.len(),
         "lfs": {"oid": sha_hex(&gguf), "size": gguf.len()}},
        {"type": "file", "path": "mmproj-test.gguf", "size": gguf.len(),
         "lfs": {"oid": sha_hex(&gguf), "size": gguf.len()}},
    ]);
    let mtp_gguf = Arc::new(fake_gguf_tensors(
        &[
            ("qwen3.expert_count", 8),
            ("qwen3.expert_used_count", 2),
            ("qwen3.nextn_predict_layers", 1),
        ],
        &["blk.0.attn_q.weight", "blk.27.nextn.eh_proj.weight"],
    ));
    let mtp_tree = json!([
        {"type": "file", "path": "mtp-moe-Q8_0.gguf", "size": mtp_gguf.len(),
         "lfs": {"oid": sha_hex(&mtp_gguf), "size": mtp_gguf.len()}},
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
                    VISION => Json(json!({"id": VISION, "gated": false, "cardData": {}})).into_response(),
                    MTP => Json(json!({"id": MTP, "gated": false, "cardData": {}})).into_response(),
                    _ => StatusCode::NOT_FOUND.into_response(),
                }
            }),
        )
        .route(
            "/api/models/{owner}/{repo}/tree/main",
            get(move |UrlPath((o, r)): UrlPath<(String, String)>| async move {
                match format!("{o}/{r}").as_str() {
                    VISION => Json(vision_tree.clone()),
                    MTP => Json(mtp_tree.clone()),
                    _ => Json(tree.clone()),
                }
            }),
        )
        .route(
            "/{owner}/{repo}/resolve/main/{*file}",
            get(move |UrlPath((o, r, _f)): UrlPath<(String, String, String)>| {
                let g = if format!("{o}/{r}") == MTP {
                    mtp_gguf.clone()
                } else {
                    g.clone()
                };
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
    /// Pause avant chaque blob (téléchargement lent).
    pub blob_delay_ms: u64,
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
    let delay = r.lock().unwrap().blob_delay_ms;
    if delay > 0 {
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
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

/// Délais courts ; pas de boucle de mise à jour automatique en fond (les
/// tests appellent `autoupdate::tick`).
pub fn test_timings() -> Timings {
    Timings {
        keepalive: Duration::from_millis(200),
        wait_limit: Duration::from_secs(20),
        probe_wait: Duration::from_secs(5),
        auto_loop: false,
        auto: AutoUpdateTimings {
            first_check: Duration::from_millis(10),
            check_every: Duration::from_secs(3600),
            idle_for: Duration::from_millis(300),
            tick: Duration::from_millis(50),
        },
        sandbox: SandboxTimings {
            ready_wait: Duration::from_secs(5),
            rpc_timeout: Duration::from_secs(5),
            idle_tick: Duration::from_millis(100),
            idle_override: None,
            pull_every: None,
            memory_turn_timeout: Duration::from_secs(600),
        },
    }
}

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
    pub headers: axum::http::HeaderMap,
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
    let headers = res.headers().clone();
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
        headers,
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
    env_with("").await
}

/// Comme `env`, le faux llama-server de l'image lancé avec les variables
/// `fake_env` (`FAKE_LOAD_MS=1500 FAKE_CHAT_SCRIPT=…`).
pub async fn env_with(fake_env: &str) -> Env {
    env_full(fake_env, false).await
}

/// `trust_proxy` = `ENGINEX_TRUST_PROXY`.
pub async fn env_full(fake_env: &str, trust_proxy: bool) -> Env {
    env_cfg(fake_env, |c| c.trust_proxy = trust_proxy).await
}

/// Comme `env_with`, `tweak` ajuste la configuration avant le démarrage.
pub async fn env_cfg(fake_env: &str, tweak: impl FnOnce(&mut Config)) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let image_bin = if fake_env.is_empty() {
        fake_bin()
    } else {
        let p = dir.path().join("llama-server-image");
        let script = format!("#!/bin/sh\n{fake_env} exec {} \"$@\"\n", fake_bin().display());
        std::fs::write(&p, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    };
    let hf = serve_hf().await;
    let (registry, reg) = serve_registry().await;
    let mut config = Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        data_dir: dir.path().join("data"),
        llama_bin: None,
        image_bin,
        nvidia_smi: "/nonexistent/nvidia-smi".into(),
        trust_proxy: false,
        models_dir: dir.path().join("models"),
        llama_port: free_port(),
        registry,
        hf_base: hf,
        timings: test_timings(),
        sandbox: SandboxConfig::default(),
    };
    tweak(&mut config);
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

/// Sert `router` sur un vrai port TCP (avec `ConnectInfo`) : `http://127.0.0.1:P`.
pub async fn serve_tcp(router: Router) -> String {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(l, router.into_make_service_with_connect_info::<SocketAddr>())
            .await
            .unwrap()
    });
    format!("http://{addr}")
}

/// Télécharge le faux GGUF, active le preset créé et attend `ready`.
pub async fn make_active_preset(e: &Env) -> i64 {
    let r = e
        .send("POST", "/api/downloads", json!({"repo": REPO, "quant": "Q8_0"}))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    e.until("/api/downloads", |v| v[0]["state"] == "done").await;
    let id = e.until("/api/presets", |v| v[0]["id"].is_i64()).await[0]["id"]
        .as_i64()
        .unwrap();
    let r = e.send("POST", &format!("/api/presets/{id}/activate"), json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    e.until("/api/engine", |v| v["state"]["state"] == "ready").await;
    e.until("/api/engine/versions", |v| v["job"]["finished"] == true).await;
    id
}

/// Télécharge le dépôt de vision (GGUF + mmproj), active son preset et attend
/// `ready`.
pub async fn make_active_vision_preset(e: &Env) -> i64 {
    let r = e
        .send(
            "POST",
            "/api/downloads",
            json!({"repo": VISION, "quant": "Q8_0", "mmproj": "mmproj-test.gguf"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    e.until("/api/downloads", |v| {
        v.as_array()
            .unwrap()
            .iter()
            .any(|d| d["repo"] == VISION && d["state"] == "done")
    })
    .await;
    let presets = e
        .until("/api/presets", |v| {
            v.as_array().unwrap().iter().any(|p| p["settings"]["mmproj"] == true)
        })
        .await;
    let id = presets
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["settings"]["mmproj"] == true)
        .unwrap()["id"]
        .as_i64()
        .unwrap();
    let r = e.send("POST", &format!("/api/presets/{id}/activate"), json!({})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    e.until("/api/engine", |v| {
        v["state"]["state"] == "ready" && v["state"]["preset_id"] == id
    })
    .await;
    e.until("/api/engine/versions", |v| v["job"]["finished"] == true).await;
    id
}

/// Crée une clé d'API et renvoie sa valeur.
pub async fn new_key(e: &Env, name: &str) -> String {
    let r = e.send("POST", "/api/api-keys", json!({"name": name})).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    r.body["key"].as_str().unwrap().to_string()
}

// ---- Sessions ----

/// Sert `router` sur `port` (choisi d'avance par `free_port`).
pub async fn serve_on(router: Router, port: u16) -> String {
    let l = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    tokio::spawn(async move {
        axum::serve(l, router.into_make_service_with_connect_info::<SocketAddr>())
            .await
            .unwrap()
    });
    format!("http://127.0.0.1:{port}")
}

pub fn contract_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/contract")
}

/// Étapes de `appserver-<nom>.jsonl` ; `edit` insère des étapes de contrôle.
pub fn script_with(name: &str, edit: impl FnOnce(&mut Vec<Value>)) -> Vec<Value> {
    let raw = std::fs::read_to_string(contract_dir().join(format!("appserver-{name}.jsonl")))
        .unwrap_or_else(|_| panic!("appserver-{name}.jsonl absent"));
    let mut steps: Vec<Value> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    edit(&mut steps);
    steps
}

/// Position de la `n`-ième étape (0 = première) qui vérifie `f`.
pub fn step_pos(steps: &[Value], n: usize, f: impl Fn(&Value) -> bool) -> usize {
    steps
        .iter()
        .enumerate()
        .filter(|(_, s)| f(s))
        .nth(n)
        .map(|(i, _)| i)
        .unwrap_or_else(|| panic!("étape {n} introuvable"))
}

pub fn is_send(method: &str) -> impl Fn(&Value) -> bool + '_ {
    move |s: &Value| s["send"]["method"] == method
}

pub fn is_expect(method: &str) -> impl Fn(&Value) -> bool + '_ {
    move |s: &Value| s["expect"] == method
}

/// Fichiers du faux app-server d'un test.
pub struct FakeAs {
    pub script: PathBuf,
    pub log: PathBuf,
    pub state: PathBuf,
}

impl FakeAs {
    pub fn log(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    /// Requêtes reçues de cette méthode.
    pub fn received(&self, method: &str) -> Vec<Value> {
        self.log()
            .into_iter()
            .filter_map(|l| l.get("received").cloned())
            .filter(|m| m["method"] == method)
            .collect()
    }

    pub fn unexpected(&self) -> Vec<Value> {
        self.log()
            .into_iter()
            .filter_map(|l| l.get("unexpected").cloned())
            .collect()
    }
}

/// EngineX avec sandboxes locales jouées par le faux app-server (`script`) ;
/// `tweak` ajuste la configuration.
pub async fn env_sessions_cfg(script: &[Value], fake_env: &str, tweak: impl FnOnce(&mut Config)) -> (Env, FakeAs) {
    let files = tempfile::tempdir().unwrap().keep();
    let fake = FakeAs {
        script: files.join("script.jsonl"),
        log: files.join("log.jsonl"),
        state: files.join("state"),
    };
    let lines: Vec<String> = script.iter().map(Value::to_string).collect();
    std::fs::write(&fake.script, lines.join("\n")).unwrap();
    let extra = vec![
        ("FAKE_APPSERVER_SCRIPT".to_string(), fake.script.display().to_string()),
        ("FAKE_APPSERVER_LOG".to_string(), fake.log.display().to_string()),
        ("FAKE_APPSERVER_STATE".to_string(), fake.state.display().to_string()),
    ];
    let e = env_cfg(fake_env, |c| {
        c.sandbox.backend = enginex_server::config::SandboxBackend::Local;
        c.sandbox.codex_bin = fake_bin();
        c.sandbox.extra_env = extra;
        tweak(c);
    })
    .await;
    (e, fake)
}

pub async fn env_sessions(script: &[Value]) -> (Env, FakeAs) {
    env_sessions_cfg(script, "", |_| {}).await
}

/// Id du topic `general/inbox`.
pub async fn inbox(e: &Env) -> i64 {
    let p = e.get("/api/projects").await.body;
    p["projects"][0]["topics"][0]["id"].as_i64().unwrap()
}

pub async fn new_session(e: &Env, approval: &str) -> String {
    let topic = inbox(e).await;
    let r = e
        .send(
            "POST",
            "/api/sessions",
            json!({"topic_id": topic, "approval": approval}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    r.body["id"].as_str().unwrap().to_string()
}

/// Flux SSE ouvert : événements `(nom, données)`.
pub struct Sse {
    rx: tokio::sync::mpsc::UnboundedReceiver<(String, Value)>,
    pub seen: Vec<(String, Value)>,
}

pub async fn sse(e: &Env, path: &str) -> Sse {
    let req = Request::builder()
        .method("GET")
        .uri(path)
        .header(header::HOST, "enginex:8090")
        .header(header::COOKIE, &e.cookie)
        .body(Body::empty())
        .unwrap();
    let res = e.router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut body = res.into_body().into_data_stream();
        let mut buf = String::new();
        use futures_util::StreamExt;
        while let Some(Ok(chunk)) = body.next().await {
            buf.push_str(&String::from_utf8_lossy(&chunk));
            while let Some(pos) = buf.find("\n\n") {
                let block: String = buf.drain(..pos + 2).collect();
                let mut name = String::new();
                let mut data = String::new();
                for line in block.lines() {
                    if let Some(n) = line.strip_prefix("event: ") {
                        name = n.to_string();
                    } else if let Some(d) = line.strip_prefix("data: ") {
                        data.push_str(d);
                    }
                }
                if name.is_empty() {
                    continue;
                }
                let v = serde_json::from_str(&data).unwrap_or(Value::Null);
                if tx.send((name, v)).is_err() {
                    return;
                }
            }
        }
    });
    Sse { rx, seen: Vec::new() }
}

impl Sse {
    /// Lit jusqu'à un événement `name` qui vérifie `f` (30 s au plus).
    pub async fn until(&mut self, name: &str, f: impl Fn(&Value) -> bool) -> Value {
        loop {
            let ev = tokio::time::timeout(Duration::from_secs(30), self.rx.recv())
                .await
                .unwrap_or_else(|_| panic!("événement {name} attendu ; vus : {:?}", self.names()))
                .expect("flux SSE fermé");
            self.seen.push(ev.clone());
            if ev.0 == name && f(&ev.1) {
                return ev.1;
            }
        }
    }

    /// Dernier événement `name` reçu jusqu'ici (sans attendre).
    pub fn last(&mut self, name: &str) -> Option<Value> {
        while let Ok(ev) = self.rx.try_recv() {
            self.seen.push(ev);
        }
        self.seen.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v.clone())
    }

    pub fn names(&self) -> Vec<String> {
        self.seen
            .iter()
            .map(|(n, v)| match v["method"].as_str() {
                Some(m) => format!("{n}:{m}"),
                None => n.clone(),
            })
            .collect()
    }
}

pub async fn turn(e: &Env, sid: &str, text: &str) -> Resp {
    e.send("POST", &format!("/api/sessions/{sid}/turns"), json!({ "text": text }))
        .await
}

/// Envoie des octets bruts avec un Content-Type.
pub async fn upload(e: &Env, path: &str, ctype: &str, bytes: Vec<u8>) -> Resp {
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header(header::HOST, "enginex:8090")
        .header(header::COOKIE, &e.cookie)
        .header(header::CONTENT_TYPE, ctype)
        .body(Body::from(bytes))
        .unwrap();
    let res = e.router.clone().oneshot(req).await.unwrap();
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Resp {
        status,
        body: serde_json::from_str(&text).unwrap_or(Value::Null),
        text,
        cookie: None,
        headers,
    }
}

pub fn png(w: u32, h: u32) -> Vec<u8> {
    let mut buf = Vec::new();
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(w, h, image::Rgb([200, 20, 20])))
        .write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)
        .unwrap();
    buf
}

// ---- WebSocket ----

pub type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Ouvre le WebSocket `path` sur `base` (`http://hôte:port`) avec le cookie de
/// session ; `origin` absent = pas d'en-tête Origin. Erreur : statut HTTP.
pub async fn ws_connect(base: &str, cookie: &str, path: &str, origin: Option<&str>) -> Result<Ws, u16> {
    use tokio_tungstenite::tungstenite::{Error, client::IntoClientRequest};
    let url = format!("{}{path}", base.replacen("http://", "ws://", 1));
    let mut req = url.into_client_request().unwrap();
    req.headers_mut().insert(header::COOKIE, cookie.parse().unwrap());
    if let Some(o) = origin {
        req.headers_mut().insert(header::ORIGIN, o.parse().unwrap());
    }
    match tokio_tungstenite::connect_async(req).await {
        Ok((ws, _)) => Ok(ws),
        Err(Error::Http(r)) => Err(r.status().as_u16()),
        Err(e) => panic!("WebSocket {path} : {e}"),
    }
}

// ---- Copies séparées ----

/// `git` sur l'hôte (tests), identité fixe.
pub fn git_in(dir: &std::path::Path, args: &[&str]) -> String {
    let o = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@localhost",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        o.status.success(),
        "git {args:?} : {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).trim().to_string()
}

/// Fait du topic `inbox` un dépôt git avec un commit (`a.txt`) ; `origin` si donné.
pub fn make_git_topic(e: &Env, origin: Option<&str>) -> PathBuf {
    let dir = e.dir.path().join("data/workspace/projects/general/topics/inbox");
    std::fs::create_dir_all(&dir).unwrap();
    git_in(&dir, &["init", "-q"]);
    std::fs::write(dir.join("a.txt"), "un\n").unwrap();
    git_in(&dir, &["add", "-A"]);
    git_in(&dir, &["commit", "-q", "-m", "départ"]);
    if let Some(o) = origin {
        git_in(&dir, &["remote", "add", "origin", o]);
    }
    dir
}

/// Session en copie séparée du topic `inbox` ; rend `(id, branche)`.
pub async fn new_copy_session(e: &Env) -> (String, String) {
    let r = e
        .send("POST", "/api/sessions", json!({"topic_id": 1, "copy": true}))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    (
        r.body["id"].as_str().unwrap().to_string(),
        r.body["copy_branch"].as_str().unwrap().to_string(),
    )
}
