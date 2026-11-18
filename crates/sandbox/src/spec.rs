//! Spec du conteneur d'une sandbox : seule fonction qui la construit (pure,
//! testée par instantané). Aucun autre montage, aucun port publié.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use bollard::models::{ContainerCreateBody, HostConfig};

use crate::Spec;

pub const LISTEN_PORT: u16 = 4500;
pub const WORKDIR: &str = "/workspace";
pub const CODEX_HOME: &str = "/home/dev/.codex";
pub const RUN_DIR: &str = "/run/enginex";
pub const ATTACHMENTS: &str = "/attachments";
pub const CACHE: &str = "/cache";

/// Réglages Docker de l'installation.
#[derive(Clone, Debug)]
pub struct DockerOpts {
    pub image: String,
    pub network: String,
    pub host_gateway: bool,
    pub memory: i64,
    /// Dossier de données vu par EngineX (`/data`).
    pub data_dir: PathBuf,
    /// Le même dossier vu par l'hôte Docker.
    pub host_data: PathBuf,
}

/// Nom du conteneur : `enginex-sbx-<8 premiers caractères de la session>`.
pub fn container_name(session_id: &str) -> String {
    let short: String = session_id.chars().take(8).collect();
    format!("enginex-sbx-{short}")
}

/// Chemin de l'hôte d'un chemin sous le dossier de données.
pub fn host_path(p: &Path, opts: &DockerOpts) -> anyhow::Result<String> {
    let rel = p
        .strip_prefix(&opts.data_dir)
        .map_err(|_| anyhow::anyhow!("{} n'est pas sous {}", p.display(), opts.data_dir.display()))?;
    Ok(opts.host_data.join(rel).display().to_string())
}

pub fn container_spec(spec: &Spec, opts: &DockerOpts) -> anyhow::Result<ContainerCreateBody> {
    let m = &spec.mounts;
    let binds = vec![
        format!("{}:{WORKDIR}:rw", host_path(&m.topic, opts)?),
        format!("{}:{CODEX_HOME}:rw", host_path(&m.codex_home, opts)?),
        format!("{}:{RUN_DIR}:ro", host_path(&m.run, opts)?),
        format!("{}:{ATTACHMENTS}:ro", host_path(&m.attachments, opts)?),
        format!("{}:{CACHE}:rw", host_path(&m.cache, opts)?),
    ];
    let mut env: Vec<String> = spec.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
    env.push(format!("CODEX_HOME={CODEX_HOME}"));
    let labels = HashMap::from([
        ("enginex.session".to_string(), spec.session_id.clone()),
        ("enginex.owner".to_string(), spec.owner.clone()),
        ("enginex.preset".to_string(), spec.preset.clone()),
    ]);
    Ok(ContainerCreateBody {
        image: Some(opts.image.clone()),
        user: Some("1000:1000".into()),
        working_dir: Some(WORKDIR.into()),
        cmd: Some(
            [
                "codex",
                "app-server",
                "--listen",
                &format!("ws://0.0.0.0:{LISTEN_PORT}"),
                "--ws-auth",
                "capability-token",
                "--ws-token-file",
                &format!("{RUN_DIR}/token"),
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        ),
        env: Some(env),
        labels: Some(labels),
        host_config: Some(HostConfig {
            binds: Some(binds),
            network_mode: Some(opts.network.clone()),
            cap_drop: Some(vec!["ALL".into()]),
            security_opt: Some(vec!["no-new-privileges:true".into()]),
            privileged: Some(false),
            pids_limit: Some(4096),
            memory: Some(opts.memory),
            auto_remove: Some(false),
            init: Some(true),
            extra_hosts: opts
                .host_gateway
                .then(|| vec!["host.docker.internal:host-gateway".into()]),
            ..Default::default()
        }),
        ..Default::default()
    })
}
