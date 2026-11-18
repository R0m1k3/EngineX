//! Instantanés (dépôt fantôme), backend local, vrai git.

mod common;

use std::{os::unix::fs::PermissionsExt, path::Path};

use common::*;
use enginex_server::snapshots::{self, Folder, SnapOpts, UndoFile};
use serde_json::{Value, json};

fn folder(e: &Env, name: &str) -> Folder {
    let work = e.dir.path().join("data/work").join(name);
    std::fs::create_dir_all(&work).unwrap();
    Folder {
        session_id: format!("sess-{name}"),
        work,
        snap: e.dir.path().join("data/snapshots").join(format!("{name}.git")),
    }
}

async fn snap(e: &Env, f: &Folder, base: Option<&str>) -> snapshots::Snap {
    snapshots::snapshot(
        &e.state,
        f,
        "test",
        SnapOpts {
            base: base.map(str::to_string),
            ..Default::default()
        },
    )
    .await
    .unwrap()
}

fn write(f: &Folder, p: &str, data: impl AsRef<[u8]>) {
    let p = f.work.join(p);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, data).unwrap();
}

fn files(c: &Value) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = c["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["path"].as_str().unwrap().to_string(),
                f["status"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    v.sort();
    v
}

fn entry<'a>(c: &'a Value, path: &str) -> &'a Value {
    c["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == path)
        .unwrap()
}

#[tokio::test]
async fn changes_are_exact() {
    let (e, _fake) = env_sessions(&[]).await;
    let f = folder(&e, "a");
    write(&f, "keep.txt", "un\ndeux\ntrois\n");
    write(&f, "gone.txt", "x");
    write(&f, "run.sh", "echo");
    let pre = snap(&e, &f, None).await;
    write(&f, "keep.txt", "un\nDEUX\ntrois\n");
    std::fs::remove_file(f.work.join("gone.txt")).unwrap();
    write(&f, "new.txt", "neuf");
    write(&f, "img.png", png(4, 4));
    std::fs::set_permissions(f.work.join("run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let post = snap(&e, &f, Some(&pre.sha)).await;
    let c = post.changes.unwrap();
    assert_eq!(
        files(&c),
        vec![
            ("gone.txt".into(), "D".into()),
            ("img.png".into(), "A".into()),
            ("keep.txt".into(), "M".into()),
            ("new.txt".into(), "A".into()),
            ("run.sh".into(), "M".into()),
        ]
    );
    assert_eq!(entry(&c, "keep.txt")["added"], 1);
    assert_eq!(entry(&c, "keep.txt")["deleted"], 1);
    assert_eq!(entry(&c, "img.png")["binary"], true);
    assert_eq!(c["truncated"], false);
    // Fichier remplacé par un lien : changement de type.
    std::fs::remove_file(f.work.join("new.txt")).unwrap();
    std::os::unix::fs::symlink("keep.txt", f.work.join("new.txt")).unwrap();
    let post2 = snap(&e, &f, Some(&post.sha)).await;
    assert_eq!(files(&post2.changes.unwrap()), vec![("new.txt".into(), "T".into())]);
}

#[tokio::test]
async fn many_changes_are_truncated() {
    let (e, _fake) = env_sessions(&[]).await;
    let f = folder(&e, "many");
    let pre = snap(&e, &f, None).await;
    for i in 0..600 {
        write(&f, &format!("d/f{i:03}.txt"), format!("{i}"));
    }
    let c = snap(&e, &f, Some(&pre.sha)).await.changes.unwrap();
    assert_eq!(c["files"].as_array().unwrap().len(), 500);
    assert_eq!(c["truncated"], true);
}

#[tokio::test]
async fn ignores_and_big_files() {
    let (e, _fake) = env_sessions(&[]).await;
    let f = folder(&e, "big");
    write(&f, ".gitignore", "secret.log\n");
    write(&f, "small.bin", vec![1u8; 1024 * 1024]);
    let pre = snap(&e, &f, None).await;
    write(&f, "node_modules/x/index.js", "x");
    write(&f, "secret.log", "x");
    write(&f, "huge.bin", vec![0u8; 60 * 1024 * 1024]);
    write(&f, "small.bin", vec![2u8; 60 * 1024 * 1024]);
    let c = snap(&e, &f, Some(&pre.sha)).await.changes.unwrap();
    assert_eq!(files(&c), vec![("small.bin".into(), "D".into())], "{c}");
    let exclude = std::fs::read_to_string(f.snap.join("info/exclude")).unwrap();
    assert!(
        exclude.contains("/huge.bin") && exclude.contains("/small.bin"),
        "{exclude}"
    );
}

#[tokio::test]
async fn nothing_from_the_folder_is_executed() {
    let (e, _fake) = env_sessions(&[]).await;
    let f = folder(&e, "evil");
    let marker = e.dir.path().join("marker");
    let m = marker.display().to_string();
    write(&f, ".gitattributes", "* filter=evil\n");
    write(&f, "a.txt", "a");
    // `.git` à la racine (dépôt de l'utilisateur) piégé.
    write(
        &f,
        ".git/config",
        format!("[filter \"evil\"]\n\tclean = touch {m}\n[core]\n\tfsmonitor = touch {m}\n"),
    );
    write(&f, ".git/HEAD", "ref: refs/heads/main\n");
    // Dépôt imbriqué piégé (dossier `.git`), avec un commit.
    let sub = f.work.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let git = |dir: &Path, args: &[&str]| {
        let st = std::process::Command::new("git")
            .current_dir(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@t"])
            .args(args)
            .env_remove("GIT_DIR")
            .status()
            .unwrap();
        assert!(st.success());
    };
    git(&sub, &["init", "-q"]);
    std::fs::write(sub.join("z"), "z").unwrap();
    git(&sub, &["add", "z"]);
    git(&sub, &["commit", "-q", "-m", "z"]);
    std::fs::write(
        sub.join(".git/config"),
        format!("[core]\n\tfsmonitor = touch {m}\n\trepositoryformatversion = 0\n"),
    )
    .unwrap();
    let hook = sub.join(".git/hooks/post-index-change");
    std::fs::write(&hook, format!("#!/bin/sh\ntouch {m}\n")).unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    // Variante `gitdir:` et dépôt imbriqué sans commit.
    let sub2 = f.work.join("sub2");
    std::fs::create_dir_all(&sub2).unwrap();
    std::fs::write(sub2.join(".git"), format!("gitdir: {}\n", sub.join(".git").display())).unwrap();
    let sub3 = f.work.join("sub3");
    std::fs::create_dir_all(&sub3).unwrap();
    git(&sub3, &["init", "-q"]);
    std::fs::write(sub3.join("w"), "w").unwrap();
    let pre = snap(&e, &f, None).await;
    write(&f, "a.txt", "b");
    std::fs::write(sub.join("z"), "zz").unwrap();
    let post = snap(&e, &f, Some(&pre.sha)).await;
    snapshots::diff(&e.state, &f, &pre.sha, &post.sha, "a.txt")
        .await
        .unwrap();
    snapshots::restore_to(&e.state, &f, &pre.sha).await.unwrap();
    assert!(!marker.exists(), "une commande du dossier a été exécutée");
    assert_eq!(std::fs::read_to_string(f.work.join("a.txt")).unwrap(), "a");
    // Le dépôt imbriqué n'est pas touché par le retour arrière.
    assert_eq!(std::fs::read_to_string(sub.join("z")).unwrap(), "zz");
}

#[tokio::test]
async fn restore_and_redo() {
    let (e, _fake) = env_sessions(&[]).await;
    let f = folder(&e, "restore");
    write(&f, ".gitignore", "*.log\n");
    write(&f, "m.txt", "avant");
    write(&f, "d.txt", "là");
    write(&f, "keep.log", "ignoré");
    let outside = e.dir.path().join("outside.txt");
    std::fs::write(&outside, "cible").unwrap();
    let pre = snap(&e, &f, None).await;
    write(&f, "m.txt", "après");
    std::fs::remove_file(f.work.join("d.txt")).unwrap();
    write(&f, "added/new.txt", "nouveau");
    write(&f, "keep.log", "ignoré 2");
    std::os::unix::fs::symlink(&outside, f.work.join("link")).unwrap();
    let undo = snapshots::restore_to(&e.state, &f, &pre.sha).await.unwrap();
    assert_eq!(std::fs::read_to_string(f.work.join("m.txt")).unwrap(), "avant");
    assert_eq!(std::fs::read_to_string(f.work.join("d.txt")).unwrap(), "là");
    assert!(!f.work.join("added/new.txt").exists());
    assert!(std::fs::symlink_metadata(f.work.join("link")).is_err(), "lien supprimé");
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "cible", "cible intacte");
    assert_eq!(std::fs::read_to_string(f.work.join("keep.log")).unwrap(), "ignoré 2");
    // « Rétablir » puis le refaire.
    let undo2 = snapshots::restore_to(&e.state, &f, &undo).await.unwrap();
    assert_eq!(std::fs::read_to_string(f.work.join("m.txt")).unwrap(), "après");
    assert!(f.work.join("added/new.txt").exists());
    assert!(
        std::fs::symlink_metadata(f.work.join("link"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    snapshots::restore_to(&e.state, &f, &undo2).await.unwrap();
    assert_eq!(std::fs::read_to_string(f.work.join("m.txt")).unwrap(), "avant");
}

#[tokio::test]
async fn undo_one_file() {
    let (e, _fake) = env_sessions(&[]).await;
    let f = folder(&e, "undo");
    write(&f, "m.txt", "avant");
    let pre = snap(&e, &f, None).await;
    write(&f, "m.txt", "après");
    write(&f, "new.txt", "nouveau");
    let post = snap(&e, &f, Some(&pre.sha)).await;
    let r = snapshots::undo_file(&e.state, &f, &pre.sha, &post.sha, "new.txt", false)
        .await
        .unwrap();
    assert!(matches!(r, UndoFile::Done(_)));
    assert!(!f.work.join("new.txt").exists());
    write(&f, "m.txt", "changé depuis");
    let r = snapshots::undo_file(&e.state, &f, &pre.sha, &post.sha, "m.txt", false)
        .await
        .unwrap();
    assert!(matches!(r, UndoFile::ChangedSince));
    assert_eq!(std::fs::read_to_string(f.work.join("m.txt")).unwrap(), "changé depuis");
    let r = snapshots::undo_file(&e.state, &f, &pre.sha, &post.sha, "m.txt", true)
        .await
        .unwrap();
    assert!(matches!(r, UndoFile::Done(_)));
    assert_eq!(std::fs::read_to_string(f.work.join("m.txt")).unwrap(), "avant");
}

#[tokio::test]
async fn diff_is_masked_and_bounded() {
    let (e, _fake) = env_sessions(&[]).await;
    let secret = "diff-secret-value-123";
    let r = e
        .send(
            "PUT",
            "/api/vault",
            json!({"scope": "global", "name": "DIFF_SECRET", "value": secret}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    let f = folder(&e, "diff");
    let pre = snap(&e, &f, None).await;
    write(&f, "s.txt", format!("token={secret}\n"));
    write(
        &f,
        "big.txt",
        "ligne de texte assez longue pour remplir\n".repeat(15_000),
    );
    let post = snap(&e, &f, Some(&pre.sha)).await;
    let (d, truncated, binary) = snapshots::diff(&e.state, &f, &pre.sha, &post.sha, "s.txt")
        .await
        .unwrap();
    assert!(!d.contains(secret) && d.contains("token=***"), "{d}");
    assert!(!truncated && !binary);
    let (d, truncated, _) = snapshots::diff(&e.state, &f, &pre.sha, &post.sha, "big.txt")
        .await
        .unwrap();
    assert!(truncated);
    assert!(d.len() <= 512 * 1024 + 16, "{}", d.len());
}

#[tokio::test]
async fn refs_are_dropped_with_the_session() {
    let (e, _fake) = env_sessions(&[]).await;
    let f = folder(&e, "refs");
    snapshots::snapshot(
        &e.state,
        &f,
        "x",
        SnapOpts {
            ref_name: Some(snapshots::green_ref(&f.session_id)),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let refs = |f: &Folder| {
        let out = std::process::Command::new("git")
            .args(["--git-dir", &f.snap.display().to_string(), "for-each-ref"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    assert!(refs(&f).contains("green"));
    snapshots::drop_refs(&e.state, &f).await.unwrap();
    assert!(refs(&f).is_empty(), "{}", refs(&f));
}

#[test]
fn rel_paths() {
    for ok in ["a", "a/b.txt", ".enginex/verify.toml", "dir/.gitignore"] {
        assert!(snapshots::valid_rel(ok), "{ok}");
    }
    for bad in [
        "",
        "/etc/passwd",
        "../x",
        "a/../b",
        "a//b",
        ".git/config",
        "a/.git",
        "./a",
    ] {
        assert!(!snapshots::valid_rel(bad), "{bad}");
    }
}

/// Un fichier qui devient lien (et disparaît) pendant les instantanés : jamais
/// le contenu de la cible, et l'instantané aboutit.
#[tokio::test]
async fn file_swapped_for_a_link_during_snapshots() {
    let (e, _fake) = env_sessions(&[]).await;
    let f = folder(&e, "race");
    let target = e.dir.path().join("cible-secrete");
    std::fs::write(&target, "CIBLE").unwrap();
    write(&f, "stable.txt", "x");
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let t = {
        let (stop, p, target) = (stop.clone(), f.work.join("piege"), target.clone());
        std::thread::spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = std::fs::remove_file(&p);
                let _ = std::fs::write(&p, "PIEGE\n");
                let _ = std::fs::remove_file(&p);
                let _ = std::os::unix::fs::symlink(&target, &p);
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        })
    };
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..20 {
        let s = snap(&e, &f, None).await;
        let ls = std::process::Command::new("git")
            .arg("--git-dir")
            .arg(&f.snap)
            .args(["ls-tree", &s.sha, "piege"])
            .output()
            .unwrap();
        let ls = String::from_utf8_lossy(&ls.stdout).into_owned();
        let Some(mode) = ls.split_whitespace().next() else {
            seen.insert("absent");
            continue;
        };
        let blob = std::process::Command::new("git")
            .arg("--git-dir")
            .arg(&f.snap)
            .args(["cat-file", "-p", &format!("{}:piege", s.sha)])
            .output()
            .unwrap()
            .stdout;
        match mode {
            "100644" => {
                // Fichier saisi vide entre sa création et son écriture : admis.
                assert!(b"PIEGE\n".starts_with(&blob), "{:?}", String::from_utf8_lossy(&blob));
                seen.insert("fichier");
            }
            "120000" => {
                assert_eq!(blob, target.as_os_str().as_encoded_bytes());
                seen.insert("lien");
            }
            m => panic!("mode {m}"),
        }
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    t.join().unwrap();
    eprintln!("course : {seen:?}");
}
