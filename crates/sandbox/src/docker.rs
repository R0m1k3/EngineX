//! Backend Docker (bollard), par le docker-socket-proxy
//! (`CONTAINERS=1 IMAGES=1 POST=1`). La spec vient de `spec::container_spec`.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Mutex,
};

use bollard::{
    API_DEFAULT_VERSION,
    errors::Error as DockerError,
    query_parameters::{
        CreateContainerOptions, CreateImageOptions, InspectContainerOptions, ListContainersOptions, LogsOptions,
        RemoveContainerOptions,
    },
};
use futures_util::StreamExt;

use crate::{
    Handle, Owned, Progress, Spec,
    spec::{ATTACHMENTS, DockerOpts, LISTEN_PORT, WORKDIR, container_name, container_spec},
};

pub struct Docker {
    client: bollard::Docker,
    opts: DockerOpts,
    /// Dernier tirage : (date Unix, erreur éventuelle).
    pub last_pull: Mutex<Option<(i64, Option<String>)>>,
}

/// État affiché dans Paramètres → Sessions.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Status {
    pub reachable: bool,
    pub error: Option<String>,
    pub image_present: bool,
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn is_404(e: &DockerError) -> bool {
    matches!(e, DockerError::DockerResponseServerError { status_code: 404, .. })
}

/// Client Docker vers `host` (`tcp://…`, `http://…` ou `unix://…`).
pub fn client(host: &str) -> anyhow::Result<bollard::Docker> {
    Ok(match host.strip_prefix("unix://") {
        Some(path) => bollard::Docker::connect_with_unix(path, 120, API_DEFAULT_VERSION)?,
        None => bollard::Docker::connect_with_http(host, 120, API_DEFAULT_VERSION)?,
    })
}

/// Chemin de l'hôte monté sur `data_dir` dans le conteneur d'EngineX
/// (inspection de son propre conteneur, nommé par le hostname).
pub async fn discover_host_data(client: &bollard::Docker, data_dir: &Path) -> anyhow::Result<PathBuf> {
    let hostname = std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_string())
        .map_err(|e| anyhow::anyhow!("hostname illisible : {e}"))?;
    let c = client
        .inspect_container(&hostname, None::<InspectContainerOptions>)
        .await
        .map_err(|e| anyhow::anyhow!("inspection du conteneur {hostname} : {e}"))?;
    let want = data_dir.display().to_string();
    c.mounts
        .unwrap_or_default()
        .into_iter()
        .find(|m| m.destination.as_deref() == Some(want.as_str()))
        .and_then(|m| m.source)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("aucun montage sur {want} dans le conteneur {hostname}"))
}

impl Docker {
    pub fn new(client: bollard::Docker, opts: DockerOpts) -> Self {
        Self {
            client,
            opts,
            last_pull: Mutex::new(None),
        }
    }

    pub fn opts(&self) -> &DockerOpts {
        &self.opts
    }

    pub async fn status(&self) -> Status {
        match self.client.ping().await {
            Ok(_) => Status {
                reachable: true,
                error: None,
                image_present: self.image_present().await,
            },
            Err(e) => Status {
                reachable: false,
                error: Some(e.to_string()),
                image_present: false,
            },
        }
    }

    pub async fn image_present(&self) -> bool {
        self.client.inspect_image(&self.opts.image).await.is_ok()
    }

    /// Tire l'image de la devbox (progression en octets).
    pub async fn pull(&self, progress: &(dyn Fn(Progress) + Send + Sync)) -> anyhow::Result<()> {
        let opts = CreateImageOptions {
            from_image: Some(self.opts.image.clone()),
            ..Default::default()
        };
        let mut layers: HashMap<String, (u64, u64)> = HashMap::new();
        let mut stream = self.client.create_image(Some(opts), None, None);
        let mut res = Ok(());
        while let Some(item) = stream.next().await {
            match item {
                Ok(info) => {
                    if let Some(e) = info.error_detail.and_then(|d| d.message) {
                        res = Err(anyhow::anyhow!("tirage de {} : {e}", self.opts.image));
                        break;
                    }
                    if let (Some(id), Some(d)) = (info.id, info.progress_detail)
                        && let (Some(c), Some(t)) = (d.current, d.total)
                    {
                        layers.insert(id, (c.max(0) as u64, t.max(0) as u64));
                        let (done, total) = layers.values().fold((0, 0), |(a, b), (c, t)| (a + c, b + t));
                        progress(Progress::Pulling { done, total });
                    }
                }
                Err(e) => {
                    res = Err(anyhow::anyhow!("tirage de {} : {e}", self.opts.image));
                    break;
                }
            }
        }
        *self.last_pull.lock().expect("verrou") = Some((now(), res.as_ref().err().map(|e| e.to_string())));
        res
    }

    pub async fn start(&self, spec: &Spec, progress: &(dyn Fn(Progress) + Send + Sync)) -> anyhow::Result<Handle> {
        if !self.image_present().await {
            self.pull(progress).await?;
        }
        progress(Progress::Starting);
        let name = container_name(&spec.session_id);
        self.stop(&spec.session_id).await?;
        let body = container_spec(spec, &self.opts)?;
        self.client
            .create_container(
                Some(CreateContainerOptions {
                    name: Some(name.clone()),
                    ..Default::default()
                }),
                body,
            )
            .await
            .map_err(|e| anyhow::anyhow!("création de {name} : {e}"))?;
        self.client
            .start_container(&name, None)
            .await
            .map_err(|e| anyhow::anyhow!("démarrage de {name} : {e}"))?;
        let c = self
            .client
            .inspect_container(&name, None::<InspectContainerOptions>)
            .await?;
        let ip = c
            .network_settings
            .and_then(|n| n.networks)
            .and_then(|n| n.get(&self.opts.network).cloned())
            .and_then(|e| e.ip_address)
            .filter(|ip| !ip.is_empty())
            .ok_or_else(|| anyhow::anyhow!("{name} n'a pas d'adresse sur le réseau {}", self.opts.network))?;
        Ok(Handle {
            ws_url: format!("ws://{ip}:{LISTEN_PORT}"),
            readyz_url: format!("http://{ip}:{LISTEN_PORT}/readyz"),
            cwd: WORKDIR.into(),
            attachments: ATTACHMENTS.into(),
        })
    }

    pub async fn stop(&self, session_id: &str) -> anyhow::Result<()> {
        let name = container_name(session_id);
        match self
            .client
            .remove_container(
                &name,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
        {
            Ok(()) => Ok(()),
            Err(e) if is_404(&e) => Ok(()),
            Err(e) => Err(anyhow::anyhow!("suppression de {name} : {e}")),
        }
    }

    pub async fn list_owned(&self, owner: &str) -> anyhow::Result<Vec<Owned>> {
        let filters = HashMap::from([("label".to_string(), vec![format!("enginex.owner={owner}")])]);
        let list = self
            .client
            .list_containers(Some(ListContainersOptions {
                all: true,
                filters: Some(filters),
                ..Default::default()
            }))
            .await?;
        Ok(list
            .into_iter()
            .filter_map(|c| {
                let labels = c.labels.unwrap_or_default();
                Some(Owned {
                    session_id: labels.get("enginex.session")?.clone(),
                    preset: labels.get("enginex.preset").cloned().unwrap_or_default(),
                })
            })
            .collect())
    }

    pub async fn logs(&self, session_id: &str) -> String {
        let name = container_name(session_id);
        let mut s = self.client.logs(
            &name,
            Some(LogsOptions {
                stdout: true,
                stderr: true,
                tail: "30".into(),
                ..Default::default()
            }),
        );
        let mut out = String::new();
        while let Some(Ok(l)) = s.next().await {
            out.push_str(&l.to_string());
        }
        crate::tail(&out, 30)
    }
}
