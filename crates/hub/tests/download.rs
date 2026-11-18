//! Téléchargement contre un serveur HTTP local qui coupe, ignore les plages
//! ou répond en erreur.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use enginex_hub::{
    download::{DownloadError, enough_space, fetch_with, free_bytes},
    hf::FileSpec,
};
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

const DELAYS: [Duration; 3] = [Duration::from_millis(10); 3];

#[derive(Clone)]
struct Srv {
    data: Arc<Vec<u8>>,
    hits: Arc<AtomicUsize>,
}

fn data() -> Vec<u8> {
    (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect()
}

fn sha(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect()
}

fn start(headers: &HeaderMap) -> usize {
    headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("bytes="))
        .and_then(|v| v.trim_end_matches('-').parse().ok())
        .unwrap_or(0)
}

async fn file(Path(mode): Path<String>, State(s): State<Srv>, headers: HeaderMap) -> Response {
    let n = s.hits.fetch_add(1, Ordering::SeqCst);
    let from = start(&headers);
    let body = s.data[from..].to_vec();
    match mode.as_str() {
        "404" => StatusCode::NOT_FOUND.into_response(),
        "503-once" if n == 0 => StatusCode::SERVICE_UNAVAILABLE.into_response(),
        // Ignore la plage : toujours tout le fichier en 200.
        "no-range" if n > 0 => (StatusCode::OK, s.data.to_vec()).into_response(),
        // Première requête : la moitié, puis coupure.
        "cut" | "no-range" if n == 0 => {
            let half = body[..body.len() / 2].to_vec();
            let stream = futures_util::stream::iter(vec![
                Ok::<_, std::io::Error>(half),
                Err(std::io::Error::other("coupure")),
            ]);
            (StatusCode::OK, Body::from_stream(stream)).into_response()
        }
        "slow" => {
            let stream = futures_util::stream::unfold(0usize, |i| async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                (i < 1000).then(|| (Ok::<_, std::io::Error>(vec![0u8; 10]), i + 1))
            });
            (StatusCode::OK, Body::from_stream(stream)).into_response()
        }
        _ if from > 0 => (StatusCode::PARTIAL_CONTENT, body).into_response(),
        _ => (StatusCode::OK, body).into_response(),
    }
}

async fn server() -> (String, Srv) {
    let s = Srv {
        data: Arc::new(data()),
        hits: Arc::new(AtomicUsize::new(0)),
    };
    let app = Router::new().route("/f/{mode}", get(file)).with_state(s.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (format!("http://{addr}"), s)
}

fn spec(sha256: String) -> FileSpec {
    FileSpec {
        name: "m.gguf".into(),
        size: 300_000,
        sha256,
    }
}

async fn get_file(mode: &str, sha256: String) -> (Result<(), DownloadError>, Vec<u8>, usize) {
    let (base, s) = server().await;
    let dir = tempfile::tempdir().unwrap();
    let part = dir.path().join("m.gguf.part");
    let r = fetch_with(
        &reqwest::Client::new(),
        &format!("{base}/f/{mode}"),
        &part,
        &spec(sha256),
        &CancellationToken::new(),
        &|_| {},
        &DELAYS,
    )
    .await;
    (
        r,
        std::fs::read(&part).unwrap_or_default(),
        s.hits.load(Ordering::SeqCst),
    )
}

#[tokio::test]
async fn nominal() {
    let (r, got, hits) = get_file("ok", sha(&data())).await;
    assert_eq!(r, Ok(()));
    assert_eq!(got, data());
    assert_eq!(hits, 1);
}

#[tokio::test]
async fn resumes_after_cut() {
    let (r, got, hits) = get_file("cut", sha(&data())).await;
    assert_eq!(r, Ok(()));
    assert_eq!(got, data());
    assert_eq!(hits, 2);
}

#[tokio::test]
async fn restarts_when_range_ignored() {
    let (r, got, _) = get_file("no-range", sha(&data())).await;
    assert_eq!(r, Ok(()));
    assert_eq!(got, data());
}

#[tokio::test]
async fn retries_on_503() {
    let (r, _, hits) = get_file("503-once", sha(&data())).await;
    assert_eq!(r, Ok(()));
    assert_eq!(hits, 2);
}

#[tokio::test]
async fn wrong_sha_and_404() {
    let (r, _, _) = get_file("ok", "00".repeat(32)).await;
    assert_eq!(r, Err(DownloadError::Sha256Mismatch));
    let (r, _, hits) = get_file("404", sha(&data())).await;
    assert_eq!((r, hits), (Err(DownloadError::Status(404)), 1));
}

#[tokio::test]
async fn resume_existing_part() {
    let (base, s) = server().await;
    let dir = tempfile::tempdir().unwrap();
    let part = dir.path().join("m.gguf.part");
    std::fs::write(&part, &data()[..100_000]).unwrap();
    let seen = std::sync::Mutex::new(Vec::new());
    let r = fetch_with(
        &reqwest::Client::new(),
        &format!("{base}/f/ok"),
        &part,
        &spec(sha(&data())),
        &CancellationToken::new(),
        &|n| seen.lock().unwrap().push(n),
        &DELAYS,
    )
    .await;
    assert_eq!(r, Ok(()));
    assert_eq!(std::fs::read(&part).unwrap(), data());
    assert_eq!(seen.lock().unwrap()[0], 100_000);
    assert_eq!(s.hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cancel() {
    let (base, _) = server().await;
    let dir = tempfile::tempdir().unwrap();
    let token = CancellationToken::new();
    let t = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        t.cancel();
    });
    let r = fetch_with(
        &reqwest::Client::new(),
        &format!("{base}/f/slow"),
        &dir.path().join("p"),
        &spec(sha(&data())),
        &token,
        &|_| {},
        &DELAYS,
    )
    .await;
    assert_eq!(r, Err(DownloadError::Cancelled));
}

#[test]
fn space() {
    assert!(enough_space(3 << 30, 1 << 30));
    assert!(!enough_space(3 << 30, (2 << 30) + 1));
    assert!(free_bytes(std::path::Path::new("/tmp")).unwrap() > 0);
}
