//! État partagé du serveur.

use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

use sqlx::SqlitePool;
use tokio::sync::Semaphore;

use crate::{config::Config, limiter::Limiter, system::SysCache};

/// Calculs argon2 simultanés au maximum (environ 19 Mio chacun).
pub const HASH_PERMITS: usize = 2;

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
}

impl AppState {
    pub fn new(config: Config, db: SqlitePool) -> Self {
        Self(Arc::new(Inner {
            config,
            db,
            limiter: Mutex::new(Limiter::default()),
            setup_code: Mutex::new(None),
            hash_permits: Arc::new(Semaphore::new(HASH_PERMITS)),
            sys_cache: tokio::sync::Mutex::new(SysCache::default()),
            started: Instant::now(),
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
}
