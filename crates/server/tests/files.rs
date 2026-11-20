//! Page Fichiers : arbre, fichier brut, aperçu texte. Rien hors du dossier
//! de la session, aucun lien suivi, en-têtes d'isolement.

mod common;

use axum::http::StatusCode;
use common::*;
use serde_json::json;

async fn env() -> (Env, String, std::path::PathBuf) {
    let (e, _fake) = env_sessions(&script_with("text", |_| {})).await;
    let sid = new_session(&e, "auto").await;
    let topic = work_dir(&e, &sid).await;
    (e, sid, topic)
}

#[tokio::test]
async fn tree_lists_one_level_dirs_first_without_git_or_escapes() {
    let (e, sid, topic) = env().await;
    std::fs::create_dir_all(topic.join("src/deep")).unwrap();
    std::fs::create_dir_all(topic.join(".git")).unwrap();
    std::fs::write(topic.join("b.txt"), "b").unwrap();
    std::fs::write(topic.join("A.txt"), "a").unwrap();
    std::fs::write(topic.join("src/main.rs"), "fn main() {}").unwrap();
    std::os::unix::fs::symlink("/etc", topic.join("etc")).unwrap();
    let r = e.get(&format!("/api/sessions/{sid}/tree")).await;
    assert_eq!(r.status, 200, "{}", r.text);
    let names: Vec<(&str, &str)> = r.body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| (v["name"].as_str().unwrap(), v["kind"].as_str().unwrap()))
        .filter(|(n, _)| !["AGENTS.md", "memory.md", "CLAUDE.md"].contains(n))
        .collect();
    assert_eq!(
        names,
        [("src", "dir"), ("A.txt", "file"), ("b.txt", "file"), ("etc", "link")]
    );
    let r = e.get(&format!("/api/sessions/{sid}/tree?path=src")).await;
    assert_eq!(r.body["entries"].as_array().unwrap().len(), 2);
    let r = e.get(&format!("/api/sessions/{sid}/tree?path=/workspace/src")).await;
    assert_eq!(r.status, 200);
    for p in ["etc", "..", "src/../..", "/etc", ".git/../../.."] {
        let r = e.get(&format!("/api/sessions/{sid}/tree?path={p}")).await;
        assert_eq!(r.status, StatusCode::NOT_FOUND, "{p}");
    }
}

#[tokio::test]
async fn tree_is_bounded() {
    let (e, sid, topic) = env().await;
    std::fs::create_dir_all(topic.join("many")).unwrap();
    for i in 0..1100 {
        std::fs::write(topic.join(format!("many/f{i:04}")), "").unwrap();
    }
    let r = e.get(&format!("/api/sessions/{sid}/tree?path=many")).await;
    assert_eq!(r.body["entries"].as_array().unwrap().len(), 1000);
    assert_eq!(r.body["truncated"], true);
}

#[tokio::test]
async fn raw_file_headers() {
    let (e, sid, topic) = env().await;
    std::fs::write(topic.join("page.html"), "<script>alert(1)</script>").unwrap();
    std::fs::write(topic.join("doc.pdf"), "%PDF-1.4").unwrap();
    std::fs::write(topic.join("img.svg"), "<svg/>").unwrap();
    std::fs::write(topic.join("photo.png"), png(2, 2)).unwrap();
    let get = |p: &str| {
        let u = format!("/api/sessions/{sid}/file?path={p}");
        let e = &e;
        async move { e.get(&u).await }
    };
    let r = get("page.html").await;
    assert_eq!(r.status, 200);
    assert_eq!(r.headers["content-type"], "application/octet-stream");
    assert!(
        r.headers["content-disposition"]
            .to_str()
            .unwrap()
            .starts_with("attachment")
    );
    assert_eq!(r.headers["x-content-type-options"], "nosniff");
    let r = get("img.svg").await;
    assert_eq!(r.headers["content-type"], "image/svg+xml");
    assert!(
        r.headers["content-security-policy"]
            .to_str()
            .unwrap()
            .ends_with("sandbox")
    );
    let r = get("doc.pdf").await;
    assert_eq!(r.headers["content-type"], "application/pdf");
    let csp = r.headers["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("default-src 'none'") && !csp.contains("sandbox"), "{csp}");
    let r = get("photo.png").await;
    assert_eq!(r.headers["content-type"], "image/png");
    assert!(r.headers.get("content-disposition").is_none());
    let r = e
        .get(&format!("/api/sessions/{sid}/file?path=photo.png&download=1"))
        .await;
    assert!(r.headers["content-disposition"].to_str().unwrap().contains("photo.png"));
    for p in ["", "src", "/etc/passwd", "../x"] {
        assert_eq!(get(p).await.status, StatusCode::NOT_FOUND, "{p}");
    }
}

#[tokio::test]
async fn big_file_is_streamed_whole() {
    let (e, sid, topic) = env().await;
    let f = std::fs::File::create(topic.join("big.bin")).unwrap();
    f.set_len(100 << 20).unwrap();
    let r = e.get(&format!("/api/sessions/{sid}/file?path=big.bin")).await;
    assert_eq!(r.status, 200);
    assert_eq!(r.headers["content-length"], (100u64 << 20).to_string());
}

#[tokio::test]
async fn text_preview_is_masked_bounded_and_detects_binary() {
    let (e, sid, topic) = env().await;
    let secret = "ghp_valeur_du_coffre";
    e.send(
        "PUT",
        "/api/vault",
        json!({"scope": "global", "name": "GH", "value": secret}),
    )
    .await;
    std::fs::write(topic.join("env.txt"), format!("TOKEN={secret}\n")).unwrap();
    std::fs::write(topic.join("long.txt"), "é".repeat(300 * 1024)).unwrap();
    std::fs::write(topic.join("bin.dat"), b"abc\0def").unwrap();
    let get = |p: &str| {
        let u = format!("/api/sessions/{sid}/file/text?path={p}");
        let e = &e;
        async move { e.get(&u).await }
    };
    let r = get("env.txt").await;
    assert_eq!(r.body["text"], "TOKEN=***\n");
    let r = get("long.txt").await;
    assert_eq!(r.body["truncated"], true);
    assert_eq!(r.body["text"].as_str().unwrap().len(), 512 * 1024);
    let r = get("bin.dat").await;
    assert_eq!(r.body["binary"], true);
    assert_eq!(r.body["text"], "");
}

/// Un dossier remplacé par un lien pendant les lectures : jamais rien de
/// l'extérieur (RESOLVE_BENEATH vérifie au moment de l'ouverture).
#[tokio::test]
async fn swapping_a_dir_for_a_link_never_leaks() {
    let (e, sid, topic) = env().await;
    let outside = e.dir.path().join("dehors");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("f.txt"), "DEHORS").unwrap();
    let d = topic.join("d");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("f.txt"), "dedans").unwrap();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let swapper = {
        let (stop, d, outside, topic) = (stop.clone(), d.clone(), outside.clone(), topic.clone());
        std::thread::spawn(move || {
            let real = topic.join("d-real");
            std::fs::rename(&d, &real).unwrap();
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                std::os::unix::fs::symlink(&outside, &d).unwrap();
                std::fs::remove_file(&d).unwrap();
                std::os::unix::fs::symlink(&real, &d).unwrap();
                std::fs::remove_file(&d).unwrap();
            }
        })
    };
    for _ in 0..300 {
        for route in ["file", "file/text"] {
            let r = e.get(&format!("/api/sessions/{sid}/{route}?path=d/f.txt")).await;
            assert!(!r.text.contains("DEHORS"), "{route} : {}", r.text);
        }
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    swapper.join().unwrap();
}
