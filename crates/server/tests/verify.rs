//! Recette de vérification : lecture, détection, écriture.

mod common;

use common::*;
use enginex_server::verify::{Recipe, Step, detect, load, parse};

fn names(s: &[Step]) -> Vec<&str> {
    s.iter().map(|s| s.name.as_str()).collect()
}

#[test]
fn parse_checks_the_recipe() {
    let s =
        parse("[[step]]\nname = \"a\"\nrun = \"true\"\n\n[[step]]\nname = \"b\"\nrun = \"x\"\ntimeout = 30\n").unwrap();
    assert_eq!(s[0].timeout, 600);
    assert_eq!(s[1].timeout, 30);
    assert!(parse("").is_err());
    assert!(parse("[[step]]\nname = \"\"\nrun = \"x\"\n").is_err());
    assert!(parse("[[step]]\nname = \"a\"\nrun = \"x\"\ntimeout = 5\n").is_err());
    assert!(parse("[[step]]\nname = \"a\"\nrun = \"x\"\ntimeout = 3601\n").is_err());
    assert!(parse(&"[[step]]\nname = \"a\"\nrun = \"x\"\n".repeat(21)).is_err());
    assert!(parse("pas du toml").is_err());
}

#[test]
fn detect_by_project_files() {
    let d = tempfile::tempdir().unwrap();
    let w = d.path();
    assert_eq!(load(w), Recipe::None);
    std::fs::write(
        w.join("package.json"),
        r#"{"scripts": {"build": "vite build", "test": "echo \"Error: no test specified\" && exit 1"}}"#,
    )
    .unwrap();
    std::fs::write(w.join("pnpm-lock.yaml"), "").unwrap();
    let s = detect(w);
    assert_eq!(names(&s), ["installation", "build"]);
    assert_eq!(s[0].run, "corepack pnpm install --frozen-lockfile");
    assert_eq!(s[1].run, "corepack pnpm run build");
    std::fs::create_dir(w.join("node_modules")).unwrap();
    std::fs::write(w.join("Cargo.toml"), "").unwrap();
    std::fs::write(w.join("go.mod"), "").unwrap();
    std::fs::write(w.join("pyproject.toml"), "[tool.pytest.ini_options]\n").unwrap();
    assert_eq!(
        names(&detect(w)),
        ["build", "cargo build", "cargo test", "go build", "go test", "pytest"]
    );
    // Un lien n'est pas suivi.
    let d2 = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(w.join("Cargo.toml"), d2.path().join("Cargo.toml")).unwrap();
    assert!(detect(d2.path()).is_empty());
}

#[test]
fn load_prefers_the_file_and_reports_errors() {
    let d = tempfile::tempdir().unwrap();
    let w = d.path();
    std::fs::write(w.join("Cargo.toml"), "").unwrap();
    assert!(matches!(load(w), Recipe::Detected(_)));
    std::fs::create_dir(w.join(".enginex")).unwrap();
    std::fs::write(
        w.join(".enginex/verify.toml"),
        "[[step]]\nname = \"x\"\nrun = \"make\"\n",
    )
    .unwrap();
    assert!(matches!(load(w), Recipe::File(s) if s[0].run == "make"));
    std::fs::write(w.join(".enginex/verify.toml"), "cassé").unwrap();
    assert!(matches!(load(w), Recipe::Invalid(e) if e.starts_with("verify.toml")));
    std::fs::write(w.join(".enginex/verify.toml"), "#".repeat(70_000)).unwrap();
    assert!(matches!(load(w), Recipe::Invalid(e) if e.contains("64 Kio")));
}

#[tokio::test]
async fn save_writes_the_detected_recipe_once() {
    let (e, _fake) = env_sessions(&[]).await;
    let sid = new_session(&e, "auto").await;
    let topic = e.dir.path().join("data/workspace/projects/general/topics/inbox");
    let r = e
        .send(
            "POST",
            &format!("/api/sessions/{sid}/verify/save"),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(r.body["error"], "no_recipe");
    std::fs::write(topic.join("go.mod"), "module x\n").unwrap();
    let r = e.get(&format!("/api/sessions/{sid}/verify")).await;
    assert_eq!(r.body["source"], "detected");
    let r = e
        .send(
            "POST",
            &format!("/api/sessions/{sid}/verify/save"),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(r.status, 200, "{}", r.text);
    let text = std::fs::read_to_string(topic.join(".enginex/verify.toml")).unwrap();
    assert_eq!(names(&parse(&text).unwrap()), ["go build", "go test"]);
    let r = e.get(&format!("/api/sessions/{sid}/verify")).await;
    assert_eq!(r.body["source"], "file");
    let r = e
        .send(
            "POST",
            &format!("/api/sessions/{sid}/verify/save"),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(r.body["error"], "recipe_exists");
}
