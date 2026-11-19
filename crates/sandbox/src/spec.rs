//! Spec du conteneur d'une sandbox : seule fonction qui la construit (pure,
//! testée par instantané). Aucun autre montage, aucun port publié.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use bollard::models::{ContainerCreateBody, HostConfig};

use crate::{HelperSpec, Spec};

pub const LISTEN_PORT: u16 = 4500;
/// Relais d'aperçu de la devbox (`docker/devbox/enginex-fwd.mjs`).
pub const FWD_PORT: u16 = 4501;
pub const FWD_SCRIPT: &str = "/usr/local/lib/enginex/fwd.mjs";
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
        format!("{}:{WORKDIR}:rw", host_path(&m.work, opts)?),
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
        // Relais d'aperçu en fond (absent d'une ancienne devbox : sans effet),
        // puis l'app-server à la place du shell.
        cmd: Some(vec![
            "sh".into(),
            "-c".into(),
            format!(
                "[ -f {FWD_SCRIPT} ] && node {FWD_SCRIPT} & exec codex app-server --listen ws://0.0.0.0:{LISTEN_PORT} \
                 --ws-auth capability-token --ws-token-file {RUN_DIR}/token"
            ),
        ]),
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

/// Nom d'un conteneur d'aide : `enginex-helper-<8 car. de la session>-<suffixe>`.
pub fn helper_name(session_id: &str, suffix: &str) -> String {
    let short: String = session_id.chars().take(8).collect();
    format!("enginex-helper-{short}-{suffix}")
}

/// Point de montage d'un dossier du conteneur d'aide.
pub fn helper_mount(var: &str) -> String {
    format!("/mnt/{}", var.to_ascii_lowercase())
}

/// Spec d'un conteneur d'aide : image devbox, même durcissement que la
/// sandbox, aucun réseau (sauf `network`), seulement les montages et
/// variables demandés.
pub fn helper_spec(spec: &HelperSpec, opts: &DockerOpts) -> anyhow::Result<ContainerCreateBody> {
    let mut binds = Vec::new();
    let mut env: Vec<String> = spec.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
    for (path, var, rw) in &spec.binds {
        let mount = helper_mount(var);
        binds.push(format!(
            "{}:{mount}:{}",
            host_path(path, opts)?,
            if *rw { "rw" } else { "ro" }
        ));
        env.push(format!("{var}={mount}"));
    }
    let labels = HashMap::from([
        ("enginex.helper".to_string(), "1".to_string()),
        ("enginex.owner".to_string(), spec.owner.clone()),
    ]);
    Ok(ContainerCreateBody {
        image: Some(opts.image.clone()),
        user: Some("1000:1000".into()),
        working_dir: Some("/".into()),
        cmd: Some(vec!["sh".into(), "-c".into(), spec.script.into()]),
        env: Some(env),
        labels: Some(labels),
        network_disabled: Some(!spec.network),
        host_config: Some(HostConfig {
            binds: Some(binds),
            network_mode: Some(if spec.network {
                opts.network.clone()
            } else {
                "none".into()
            }),
            cap_drop: Some(vec!["ALL".into()]),
            security_opt: Some(vec!["no-new-privileges:true".into()]),
            privileged: Some(false),
            pids_limit: Some(512),
            memory: Some(opts.memory),
            auto_remove: Some(false),
            init: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    })
}
