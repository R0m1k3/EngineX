//! Mise à jour de llama.cpp contre un registre OCI local, couches fabriquées.

mod common;

use std::{collections::HashMap, path::Path, sync::Arc};

use axum::{
    Json, Router,
    extract::{Path as UrlPath, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use enginex_engine::{
    oci::{self, Registry, UpdateError},
    update,
};
use sha2::{Digest, Sha256};

enum E<'a> {
    File(&'a str, &'a [u8]),
    Link(&'a str, &'a str),
}

fn layer(entries: &[E]) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut b = tar::Builder::new(gz);
    for e in entries {
        let mut h = tar::Header::new_gnu();
        match e {
            E::File(name, data) => {
                h.set_entry_type(tar::EntryType::Regular);
                h.set_size(data.len() as u64);
                h.set_mode(0o755);
                b.append_data(&mut h, name, *data).unwrap();
            }
            E::Link(name, target) => {
                h.set_entry_type(tar::EntryType::Symlink);
                h.set_size(0);
                h.set_mode(0o777);
                b.append_link(&mut h, name, target).unwrap();
            }
        }
    }
    b.into_inner().unwrap().finish().unwrap()
}

fn digest(b: &[u8]) -> String {
    format!(
        "sha256:{}",
        Sha256::digest(b).iter().map(|x| format!("{x:02x}")).collect::<String>()
    )
}

#[derive(Default)]
struct Reg {
    manifests: HashMap<String, serde_json::Value>,
    blobs: HashMap<String, Vec<u8>>,
}

async fn manifest(State(r): State<Arc<Reg>>, UrlPath(reference): UrlPath<String>) -> Response {
    match r.manifests.get(&reference) {
        Some(m) => Json(m.clone()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn blob(State(r): State<Arc<Reg>>, UrlPath(d): UrlPath<String>) -> Response {
    match r.blobs.get(&d) {
        Some(b) => b.clone().into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// Registre avec `server-cuda` (index → manifeste amd64) au build `build`.
/// `corrupt` : la couche servie diffère de son digest.
async fn serve(build: &str, layers: Vec<Vec<u8>>, corrupt: bool) -> String {
    let mut reg = Reg::default();
    let mut descs = Vec::new();
    for l in layers {
        let d = digest(&l);
        descs.push(serde_json::json!({"mediaType": "application/vnd.oci.image.layer.v1.tar+gzip", "digest": d, "size": l.len()}));
        let mut served = l;
        if corrupt {
            served.push(0);
        }
        reg.blobs.insert(d, served);
    }
    // Couche géante (runtime CUDA) : jamais téléchargée.
    descs.insert(0, serde_json::json!({"digest": "sha256:00", "size": 3u64 << 30}));
    let image = serde_json::json!({"schemaVersion": 2, "layers": descs});
    let index = serde_json::json!({
        "schemaVersion": 2,
        "manifests": [
            {"digest": "sha256:arm", "platform": {"os": "linux", "architecture": "arm64"}},
            {"digest": "sha256:amd", "platform": {"os": "linux", "architecture": "amd64"}}
        ],
        "annotations": {"org.opencontainers.image.version": build}
    });
    reg.manifests.insert("server-cuda".into(), index.clone());
    reg.manifests.insert(format!("server-cuda-{build}"), index);
    reg.manifests.insert("sha256:amd".into(), image);
    let app = Router::new()
        .route("/token", get(|| async { Json(serde_json::json!({"token": "anon"})) }))
        .route("/v2/ggml-org/llama.cpp/manifests/{reference}", get(manifest))
        .route("/v2/ggml-org/llama.cpp/blobs/{digest}", get(blob))
        .with_state(Arc::new(reg));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}")
}

/// Script `llama-server` qui lance le faux avec des variables imposées.
fn script(env: &str) -> Vec<u8> {
    format!("#!/bin/sh\n{env} exec {} \"$@\"\n", common::fake_bin().display()).into_bytes()
}

const LIBS: [&str; 5] = [
    "libggml.so.0",
    "libggml-base.so.0",
    "libggml-cuda.so",
    "libmtmd.so.0",
    "libllama.so.0",
];

fn full(env: &str) -> Vec<u8> {
    let s = script(env);
    let mut e: Vec<E> = vec![E::File("app/llama-server", &s)];
    for l in LIBS {
        e.push(E::File(Box::leak(format!("app/{l}").into_boxed_str()), b"elf"));
    }
    e.push(E::Link("app/libllama.so", "libllama.so.0"));
    e.push(E::File("usr/lib/autre.so", b"hors app"));
    layer(&e)
}

async fn install(base: &str, dir: &Path) -> Result<std::path::PathBuf, UpdateError> {
    let r = Registry::new(reqwest::Client::new(), base);
    let rel = r.latest("server-cuda").await?;
    r.install(&rel, dir, &|_, _| {}).await
}

#[tokio::test]
async fn latest_reads_version_and_skips_big_layers() {
    let base = serve("b12500", vec![full("")], false).await;
    let rel = Registry::new(reqwest::Client::new(), &base)
        .latest("server-cuda")
        .await
        .unwrap();
    assert_eq!(rel.build, "b12500");
    assert_eq!(rel.tag, "server-cuda-b12500");
    assert_eq!(rel.manifest_digest, "sha256:amd");
    assert_eq!(rel.layers.len(), 2);
}

#[tokio::test]
async fn installs_flat_app_files_and_passes_tests() {
    let dir = tempfile::tempdir().unwrap();
    let base = serve("b12464", vec![full("")], false).await;
    let bin = install(&base, dir.path()).await.unwrap();
    assert_eq!(bin, dir.path().join("server-cuda-b12464/llama-server"));
    let d = dir.path().join("server-cuda-b12464");
    assert_eq!(
        std::fs::read_link(d.join("libllama.so")).unwrap(),
        Path::new("libllama.so.0")
    );
    assert!(!d.join("autre.so").exists());
    assert!(!dir.path().join("server-cuda-b12464.part").exists());
    let rep = update::test_engine(&bin, Some("b12464"), 2).await.unwrap();
    assert_eq!(rep.gpus, 2);
    assert!(rep.has_fit);
}

#[tokio::test]
async fn wrong_digest_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let base = serve("b12464", vec![full("")], true).await;
    assert_eq!(
        install(&base, dir.path()).await.unwrap_err(),
        UpdateError::DigestMismatch
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn whiteout_in_upper_layer_removes_file() {
    let dir = tempfile::tempdir().unwrap();
    let upper = layer(&[E::File("app/.wh.libmtmd.so.0", b"")]);
    let base = serve("b12464", vec![full(""), upper], false).await;
    match install(&base, dir.path()).await.unwrap_err() {
        UpdateError::Incomplete(m) => assert_eq!(m, vec!["libmtmd.so".to_string()]),
        e => panic!("{e:?}"),
    }
    assert!(!dir.path().join("server-cuda-b12464.part").exists());
    assert!(!dir.path().join("server-cuda-b12464").exists());
}

#[tokio::test]
async fn link_out_of_folder_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let bad = layer(&[E::Link("app/libx.so", "../etc/passwd")]);
    let base = serve("b12464", vec![full(""), bad], false).await;
    assert!(matches!(
        install(&base, dir.path()).await.unwrap_err(),
        UpdateError::BadLink(_)
    ));
    assert!(!dir.path().join("server-cuda-b12464.part").exists());
}

#[tokio::test]
async fn incomplete_set_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let s = script("");
    let base = serve("b12464", vec![layer(&[E::File("app/llama-server", &s)])], false).await;
    match install(&base, dir.path()).await.unwrap_err() {
        UpdateError::Incomplete(m) => assert!(m.contains(&"libggml-cuda.so".to_string())),
        e => panic!("{e:?}"),
    }
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn engine_tests_fail_on_one_gpu_no_fit_or_wrong_build() {
    for (env, build) in [
        ("FAKE_GPUS=1", "b12464"),
        ("FAKE_NO_FIT=1", "b12464"),
        ("FAKE_BUILD=12000", "b12464"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let base = serve(build, vec![full(env)], false).await;
        let bin = install(&base, dir.path()).await.unwrap();
        let e = update::test_engine(&bin, Some(build), 2).await.unwrap_err();
        assert!(matches!(e, UpdateError::TestFailed(_)), "{env} : {e:?}");
    }
}

#[test]
fn apply_layer_upper_wins() {
    let dir = tempfile::tempdir().unwrap();
    oci::apply_layer(&layer(&[E::File("app/a", b"1")])[..], dir.path()).unwrap();
    oci::apply_layer(&layer(&[E::File("./app/a", b"2")])[..], dir.path()).unwrap();
    assert_eq!(std::fs::read(dir.path().join("a")).unwrap(), b"2");
}
