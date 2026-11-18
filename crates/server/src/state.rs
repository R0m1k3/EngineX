//! État partagé du serveur.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
    time::{Duration, Instant},
};

use enginex_engine::{
    oci::{Registry, Release},
    supervisor::{Supervisor, SupervisorConfig},
};
use enginex_hub::hf::Hf;
use sqlx::SqlitePool;
use tokio::sync::{RwLock, Semaphore};

use crate::{config::Config, engine_api::JobState, limiter::Limiter, models::DownloadQueue, system::SysCache};

/// Calculs argon2 simultanés au maximum (environ 19 Mio chacun).
pub const HASH_PERMITS: usize = 2;
/// Échecs de clé d'API permis par IP en 15 min sur `/v1`.
pub const V1_MAX_FAILURES: u32 = 20;
/// Délai de chargement d'un modèle (un 27B sur disque lent).
pub const READY_TIMEOUT: Duration = Duration::from_secs(600);

/// Résultat des sondes faites quand le moteur devient prêt.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct EngineProbe {
    pub preset_id: i64,
    pub n_ctx: Option<u64>,
    pub tools_ok: Option<bool>,
    pub build: Option<String>,
}

#[derive(Clone)]
pub struct AppState(pub(crate) Arc<Inner>);

pub(crate) struct Inner {
    pub config: Config,
    pub db: SqlitePool,
    pub limiter: Mutex<Limiter>,
    pub setup_code: Mutex<Option<String>>,
    pub hash_permits: Arc<Semaphore>,
    pub sys_cache: tokio::sync::Mutex<SysCache>,
    pub started: Instant,
    pub http: reqwest::Client,
    pub hf: Hf,
    pub registry: Registry,
    pub supervisor: Supervisor,
    pub downloads: DownloadQueue,
    /// VRAM prise par d'autres programmes (Mio par carte), avec l'heure de mesure.
    pub vram_others: Mutex<Option<(i64, Vec<u64>)>>,
    /// Installation ou bascule de moteur en cours (ou la dernière terminée).
    pub engine_job: Mutex<Option<JobState>>,
    /// Moteur de secours : le moteur actif a échoué au lancement d'EngineX.
    pub fallback: AtomicBool,
    pub latest: Mutex<Option<Release>>,
    pub probe: Mutex<Option<EngineProbe>>,
    /// Porte de service : chaque requête `/v1` la prend en lecture jusqu'à la
    /// fin de son flux ; tout chargement ou arrêt de llama-server la prend en
    /// écriture jusqu'à `Ready`/`Failed`/`Stopped`. Toujours après `begin_job`.
    pub serve_gate: Arc<RwLock<()>>,
    /// Fin de la dernière requête `/v1` (ms Unix ; 0 = jamais).
    pub last_done_ms: AtomicI64,
    pub auto: crate::autoupdate::AutoState,
    /// Échecs de clé d'API par IP, séparé de celui de la connexion.
    pub v1_limiter: Mutex<Limiter>,
    /// Réponses de Hugging Face gardées un moment (clé → date, valeur).
    pub hf_cache: Mutex<HashMap<String, (Instant, serde_json::Value)>>,
}

impl AppState {
    pub fn new(config: Config, db: SqlitePool) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .user_agent(concat!("EngineX/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("client HTTP");
        let supervisor = Supervisor::new(SupervisorConfig {
            port: config.llama_port,
            log_path: config.data_dir.join("logs/llama-server.log"),
            nvidia_smi: config.nvidia_smi.clone(),
            ready_timeout: READY_TIMEOUT,
        });
        Self(Arc::new(Inner {
            hf: Hf::new(http.clone(), &config.hf_base),
            registry: Registry::new(http.clone(), &config.registry),
            http,
            supervisor,
            config,
            db,
            limiter: Mutex::new(Limiter::default()),
            setup_code: Mutex::new(None),
            hash_permits: Arc::new(Semaphore::new(HASH_PERMITS)),
            sys_cache: tokio::sync::Mutex::new(SysCache::default()),
            started: Instant::now(),
            downloads: DownloadQueue::default(),
            vram_others: Mutex::new(None),
            engine_job: Mutex::new(None),
            fallback: AtomicBool::new(false),
            latest: Mutex::new(None),
            probe: Mutex::new(None),
            hf_cache: Mutex::new(HashMap::new()),
            serve_gate: Arc::new(RwLock::new(())),
            last_done_ms: AtomicI64::new(0),
            auto: Default::default(),
            v1_limiter: Mutex::new(Limiter::new(V1_MAX_FAILURES, crate::limiter::WINDOW)),
        }))
    }

    /// Le code de configuration en cours, s'il y en a un (sert aussi aux tests).
    pub fn setup_code(&self) -> Option<String> {
        self.0.setup_code.lock().expect("verrou").clone()
    }

    pub fn db(&self) -> &SqlitePool {
        &self.0.db
    }

    pub fn config(&self) -> &Config {
        &self.0.config
    }

    pub fn supervisor(&self) -> &Supervisor {
        &self.0.supervisor
    }

    pub fn serve_gate(&self) -> &Arc<RwLock<()>> {
        &self.0.serve_gate
    }

    pub fn last_done_ms(&self) -> i64 {
        self.0.last_done_ms.load(Ordering::SeqCst)
    }

    pub(crate) fn mark_done(&self) {
        self.0.last_done_ms.store(now_ms(), Ordering::SeqCst);
    }

    /// Tâches de fond : mesure de la VRAM, test du moteur, relance du preset
    /// actif, reprise des téléchargements. Appelée une fois par `main.rs`.
    pub async fn start_background(&self) -> anyhow::Result<()> {
        crate::engine_api::measure_vram_others(self).await;
        crate::engine_api::boot(self).await?;
        crate::models::boot(self).await?;
        if self.config().timings.auto_loop {
            tokio::spawn(crate::autoupdate::run(self.clone()));
        }
        Ok(())
    }
}

/// Secondes Unix.
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Millisecondes Unix.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
