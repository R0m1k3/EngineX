//! Page Fichiers : tout le workspace, par descripteurs (liens jamais suivis),
//! dossiers gérés protégés, conflit d'écriture, envoi, zip, disque.

mod common;

use axum::http::StatusCode;
use common::{env, upload};
use serde_json::json;

fn enc(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~/".contains(&b) {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

fn ws(e: &common::Env) -> std::path::PathBuf {
    e.dir.path().join("data/workspace")
}

#[tokio::test]
async fn tree_text_and_conflict() {
    let e = env().await;
    let t = e.get("/api/files/tree").await;
    assert_eq!(t.status, StatusCode::OK, "{}", t.text);
    let names: Vec<&str> = t.body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"projects"), "{names:?}");
    let projects = t.body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == "projects")
        .unwrap();
    assert_eq!(projects["managed"], true);

    let topic = "projects/general/topics/inbox";
    let path = format!("{topic}/notes/a.md");
    // Création (dossier d'abord), puis relecture.
    assert_eq!(
        e.send("POST", "/api/files/mkdir", json!({"path": format!("{topic}/notes")}))
            .await
            .status,
        StatusCode::OK
    );
    let w = e
        .send("PUT", "/api/files/text", json!({"path": path, "text": "# Bonjour\n"}))
        .await;
    assert_eq!(w.status, StatusCode::OK, "{}", w.text);
    let v1 = w.body["version"].as_i64().unwrap();
    // Recréer le même fichier : refusé.
    let again = e
        .send("PUT", "/api/files/text", json!({"path": path, "text": "x"}))
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    let r = e.get(&format!("/api/files/text?path={}", enc(&path))).await;
    assert_eq!(r.body["text"], "# Bonjour\n");
    assert_eq!(r.body["version"], v1);
    assert_eq!(r.body["readonly"], false);
    assert!(v1 < 1 << 53, "version exacte en JavaScript : {v1}");

    // Modifié ailleurs entre-temps : 409 file_changed.
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(ws(&e).join(&path), "changé par la sandbox").unwrap();
    let stale = e
        .send(
            "PUT",
            "/api/files/text",
            json!({"path": path, "text": "mon texte", "version": v1}),
        )
        .await;
    assert_eq!(stale.status, StatusCode::CONFLICT);
    assert_eq!(stale.body["error"], "file_changed");
    let r = e.get(&format!("/api/files/text?path={}", enc(&path))).await;
    let ok = e
        .send(
            "PUT",
            "/api/files/text",
            json!({"path": path, "text": "mon texte", "version": r.body["version"]}),
        )
        .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text);
    assert_eq!(std::fs::read_to_string(ws(&e).join(&path)).unwrap(), "mon texte");
    // Aucun fichier temporaire laissé.
    let left: Vec<_> = std::fs::read_dir(ws(&e).join(topic).join("notes"))
        .unwrap()
        .flatten()
        .map(|d| d.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, vec!["a.md"]);
}

#[tokio::test]
async fn managed_folders_and_memory_are_protected() {
    let e = env().await;
    for p in [
        "projects",
        "projects/general",
        "projects/general/topics",
        "projects/general/topics/inbox",
        "memory",
        "skills",
    ] {
        let r = e.send("POST", "/api/files/delete", json!({"paths": [p]})).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "{p}");
        let r = e
            .send("POST", "/api/files/move", json!({"from": p, "to": "ailleurs"}))
            .await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "{p}");
    }
    let r = e
        .send("POST", "/api/files/mkdir", json!({"path": "projects/nouveau"}))
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = e
        .send(
            "PUT",
            "/api/files/text",
            json!({"path": "memory/memory.md", "text": "x"}),
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    // `..` et `.git` : introuvables.
    for p in ["../data", "projects/general/topics/inbox/.git/config", "/etc/passwd"] {
        let r = e.get(&format!("/api/files/text?path={}", enc(p))).await;
        assert_eq!(r.status, StatusCode::NOT_FOUND, "{p}");
    }
}

#[tokio::test]
async fn symlinks_are_never_followed() {
    let e = env().await;
    let topic = ws(&e).join("projects/general/topics/inbox");
    let secret = e.dir.path().join("secret.txt");
    std::fs::write(&secret, "clé maîtresse").unwrap();
    std::os::unix::fs::symlink(&secret, topic.join("lien.txt")).unwrap();
    std::os::unix::fs::symlink(e.dir.path(), topic.join("lien-dossier")).unwrap();
    let base = "projects/general/topics/inbox";
    for p in ["lien.txt", "lien-dossier/secret.txt"] {
        let r = e
            .get(&format!("/api/files/text?path={}", enc(&format!("{base}/{p}"))))
            .await;
        assert_eq!(r.status, StatusCode::NOT_FOUND, "{p}");
        let r = e
            .get(&format!("/api/files/raw?path={}", enc(&format!("{base}/{p}"))))
            .await;
        assert_eq!(r.status, StatusCode::NOT_FOUND, "{p}");
    }
    // Écrire au travers d'un lien : refusé, la cible reste intacte.
    let r = e
        .send(
            "PUT",
            "/api/files/text",
            json!({"path": format!("{base}/lien-dossier/secret.txt"), "text": "x", "version": 0}),
        )
        .await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    // Le lien lui-même n'est pas remplacé par une écriture.
    let r = e
        .send(
            "PUT",
            "/api/files/text",
            json!({"path": format!("{base}/lien.txt"), "text": "x", "version": 0}),
        )
        .await;
    assert_ne!(r.status, StatusCode::OK);
    // Zip d'un dossier contenant des liens : liens ignorés.
    std::fs::write(topic.join("vrai.txt"), "contenu").unwrap();
    let z = e.get(&format!("/api/files/zip?paths={}", enc(base))).await;
    assert_eq!(z.status, StatusCode::OK);
    assert_eq!(&z.text.as_bytes()[..2], b"PK");
    assert!(z.text.contains("inbox/vrai.txt"));
    assert!(!z.text.contains("secret.txt"));
    // Supprimer le lien vers un dossier supprime le lien, pas le dossier.
    let r = e
        .send(
            "POST",
            "/api/files/delete",
            json!({"paths": [format!("{base}/lien-dossier"), format!("{base}/lien.txt")]}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(r.body["deleted"], 2);
    assert!(secret.exists());
    assert!(!topic.join("lien-dossier").exists());
}

#[tokio::test]
async fn upload_move_delete_and_usage() {
    let e = env().await;
    let base = "projects/general/topics/inbox";
    let r = upload(
        &e,
        &format!("/api/files/upload?path={}", enc(&format!("{base}/photo.bin"))),
        "application/octet-stream",
        vec![7u8; 300_000],
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(
        std::fs::read(ws(&e).join(base).join("photo.bin")).unwrap().len(),
        300_000
    );
    // Même nom sans `overwrite` : refusé ; avec : remplacé.
    let r = upload(
        &e,
        &format!("/api/files/upload?path={}", enc(&format!("{base}/photo.bin"))),
        "application/octet-stream",
        vec![1u8; 10],
    )
    .await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    let r = upload(
        &e,
        &format!(
            "/api/files/upload?path={}&overwrite=true",
            enc(&format!("{base}/photo.bin"))
        ),
        "application/octet-stream",
        vec![1u8; 10],
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert_eq!(
        std::fs::read(ws(&e).join(base).join("photo.bin")).unwrap(),
        vec![1u8; 10]
    );

    // Déplacer dans un sous-dossier, sans écraser.
    e.send("POST", "/api/files/mkdir", json!({"path": format!("{base}/rangé")}))
        .await;
    std::fs::write(ws(&e).join(base).join("rangé/occupé.bin"), "x").unwrap();
    let r = e
        .send(
            "POST",
            "/api/files/move",
            json!({"from": format!("{base}/photo.bin"), "to": format!("{base}/rangé/occupé.bin")}),
        )
        .await;
    assert_eq!(r.status, StatusCode::CONFLICT);
    let r = e
        .send(
            "POST",
            "/api/files/move",
            json!({"from": format!("{base}/photo.bin"), "to": format!("{base}/rangé/photo.bin")}),
        )
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert!(ws(&e).join(base).join("rangé/photo.bin").exists());

    // Supprimer un dossier non vide.
    let r = e
        .send("POST", "/api/files/delete", json!({"paths": [format!("{base}/rangé")]}))
        .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    assert!(!ws(&e).join(base).join("rangé").exists());

    let u = e.get("/api/files/usage").await;
    assert_eq!(u.status, StatusCode::OK, "{}", u.text);
    assert!(u.body["total_bytes"].as_u64().unwrap() > 0);
    assert_eq!(u.body["alert_pct"], 90);
    let s = e
        .send("PUT", "/api/settings/sessions", json!({"disk_alert_pct": 50}))
        .await;
    assert_eq!(s.status, StatusCode::OK, "{}", s.text);
    assert_eq!(s.body["disk_alert_pct"], 50);
    assert_eq!(
        e.send("PUT", "/api/settings/sessions", json!({"disk_alert_pct": 20}))
            .await
            .status,
        StatusCode::BAD_REQUEST
    );
}
