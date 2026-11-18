//! Client Hugging Face contre un serveur local qui sert des réponses réelles
//! enregistrées le 2026-11-18 (`tests/fixtures/hf/`).

use axum::{
    Router,
    extract::Path,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use enginex_hub::hf::{Hf, HubError};

fn fixture(name: &str) -> String {
    std::fs::read_to_string(format!("{}/tests/fixtures/hf/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

/// GGUF dont les métadonnées font `pad` octets de plus (gabarit de chat).
fn gguf(pad: usize) -> Vec<u8> {
    let mut kv = Vec::new();
    let mut put = |k: &str, t: u32, v: &[u8]| {
        kv.extend((k.len() as u64).to_le_bytes());
        kv.extend(k.as_bytes());
        kv.extend(t.to_le_bytes());
        kv.extend(v);
    };
    let s = |x: &str| [(x.len() as u64).to_le_bytes().to_vec(), x.as_bytes().to_vec()].concat();
    put("general.architecture", 8, &s("qwen3"));
    put("qwen3.block_count", 4, &28u32.to_le_bytes());
    put("tokenizer.chat_template", 8, &s(&"x".repeat(pad)));
    let mut b = b"GGUF".to_vec();
    b.extend(3u32.to_le_bytes());
    b.extend(0u64.to_le_bytes());
    b.extend(3u64.to_le_bytes());
    b.extend(kv);
    b.extend(vec![0u8; 1 << 20]);
    b
}

fn range_end(h: &HeaderMap) -> Option<usize> {
    h.get(header::RANGE)?
        .to_str()
        .ok()?
        .strip_prefix("bytes=0-")?
        .parse()
        .ok()
}

async fn file(Path((owner, name, kind, file)): Path<(String, String, String, String)>, h: HeaderMap) -> Response {
    match (owner.as_str(), name.as_str(), kind.as_str(), file.as_str()) {
        (_, _, "raw", "README.md") if name == "Qwen3-0.6B-GGUF" => std::fs::read_to_string(format!(
            "{}/tests/cards/unsloth-qwen3-0.6b-gguf.md",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
        .into_response(),
        ("Qwen", "Qwen3-0.6B", "raw", "generation_config.json") => {
            fixture("generation_config-qwen3-0.6b.json").into_response()
        }
        ("gated", _, _, _) => StatusCode::UNAUTHORIZED.into_response(),
        (_, _, "resolve", f) => {
            let data = gguf(if f.contains("big") { 20 << 20 } else { 100 });
            let end = range_end(&h).map_or(data.len(), |e| (e + 1).min(data.len()));
            (StatusCode::PARTIAL_CONTENT, data[..end].to_vec()).into_response()
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn hf() -> Hf {
    let app = Router::new()
        .route(
            "/api/models",
            get(|| async { ([(header::CONTENT_TYPE, "application/json")], fixture("search.json")) }),
        )
        .route(
            "/api/models/{owner}/{name}",
            get(|Path((o, n)): Path<(String, String)>| async move {
                if (o.as_str(), n.as_str()) == ("unsloth", "Qwen3-0.6B-GGUF") {
                    (
                        [(header::CONTENT_TYPE, "application/json")],
                        fixture("model-unsloth-qwen3-0.6b.json"),
                    )
                        .into_response()
                } else {
                    StatusCode::NOT_FOUND.into_response()
                }
            }),
        )
        .route(
            "/api/models/{owner}/{name}/tree/main",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "application/json")],
                    fixture("tree-unsloth-qwen3-0.6b.json"),
                )
            }),
        )
        .route("/{owner}/{name}/{kind}/main/{file}", get(file));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    Hf::new(reqwest::Client::new(), format!("http://{addr}/"))
}

#[tokio::test]
async fn search_and_repo() {
    let hf = hf().await;
    let hits = hf.search("qwen3-0.6b").await.unwrap();
    assert_eq!(hits.len(), 3);
    assert!(hits.iter().all(|h| !h.gated && h.updated.is_some()));
    let r = hf.repo("unsloth/Qwen3-0.6B-GGUF").await.unwrap();
    assert_eq!(r.base_model.as_deref(), Some("Qwen/Qwen3-0.6B"));
    assert!(!r.gated);
    let q8 = r.groups.iter().find(|g| g.quant == "Q8_0").unwrap();
    assert_eq!(q8.files.len(), 1);
    assert_eq!(q8.files[0].name, "Qwen3-0.6B-Q8_0.gguf");
    assert_eq!(q8.files[0].sha256.len(), 64);
    assert!(q8.total_size > 600_000_000);
    assert!(r.groups.iter().any(|g| g.quant == "UD-Q4_K_XL"));
    assert_eq!(hf.repo("x/absent").await, Err(HubError::Status(404)));
}

#[tokio::test]
async fn header_by_range() {
    let hf = hf().await;
    assert_eq!(hf.header("o/n", "m.gguf").await.unwrap().block_count, Some(28));
    // Métadonnées de plus de 16 Mio : deuxième lecture, plus large.
    assert_eq!(hf.header("o/n", "big.gguf").await.unwrap().arch, "qwen3");
    assert_eq!(hf.header("gated/n", "m.gguf").await, Err(HubError::Gated));
}

#[tokio::test]
async fn readme_and_generation_config() {
    let hf = hf().await;
    assert!(
        hf.readme("unsloth/Qwen3-0.6B-GGUF")
            .await
            .unwrap()
            .unwrap()
            .contains("Temperature=0.6")
    );
    assert_eq!(hf.readme("o/absent").await, Ok(None));
    let gc = hf.generation_config("Qwen/Qwen3-0.6B").await.unwrap().unwrap();
    assert_eq!(gc["top_k"], 20);
    assert_eq!(
        hf.base_model("unsloth/Qwen3-0.6B-GGUF").await.unwrap().as_deref(),
        Some("Qwen/Qwen3-0.6B")
    );
}
