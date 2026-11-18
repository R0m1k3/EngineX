//! Outils communs des tests : le faux llama-server.
#![allow(dead_code)]

use std::path::PathBuf;

/// `target/debug/fake-llama-server`, construit par `cargo build -p enginex-fake-llama`.
pub fn fake_bin() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    // Le test tourne dans target/<profil>/deps/ (ou plus bas selon cargo).
    exe.ancestors()
        .map(|d| d.join("fake-llama-server"))
        .find(|b| b.is_file())
        .unwrap_or_else(|| {
            panic!(
                "faux llama-server absent : lancer `cargo build -p enginex-fake-llama` ({})",
                exe.display()
            )
        })
}

/// Un port libre sur 127.0.0.1.
pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Script shell qui lance le faux binaire avec des variables imposées
/// (pour qu'un binaire précis se comporte autrement).
pub fn wrapper(dir: &std::path::Path, env: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).unwrap();
    let p = dir.join("llama-server");
    std::fs::write(&p, format!("#!/bin/sh\n{env} exec {} \"$@\"\n", fake_bin().display())).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    p
}
