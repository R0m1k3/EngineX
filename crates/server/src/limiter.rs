//! Garde contre la force brute : un compteur par IP sur 15 minutes.
//! Adapté de Presencia (`server/src/limiter.rs`) : une seule clé (l'IP),
//! pas de compteur global (il permettrait à n'importe qui de bloquer
//! l'unique utilisateur).

use std::{
    hash::{BuildHasher, RandomState},
    time::{Duration, Instant},
};

use hashlink::LinkedHashMap;

pub const WINDOW: Duration = Duration::from_secs(15 * 60);
pub const MAX_PER_IP: u32 = 5;
/// Borne la mémoire quel que soit le volume d'essais.
pub const MAX_KEYS: usize = 50_000;

struct Failures {
    count: u32,
    since: Instant,
}

/// Clés dans l'ordre d'insertion : la plus ancienne fenêtre part en premier
/// quand la table est pleine. L'IP est hachée avec une clé aléatoire.
pub struct Limiter {
    map: LinkedHashMap<u64, Failures>,
    hasher: RandomState,
    max: u32,
    window: Duration,
}

/// Seuils de la connexion à l'UI.
impl Default for Limiter {
    fn default() -> Self {
        Self::new(MAX_PER_IP, WINDOW)
    }
}

impl Limiter {
    pub fn new(max: u32, window: Duration) -> Self {
        Self {
            map: LinkedHashMap::new(),
            hasher: RandomState::new(),
            max,
            window,
        }
    }

    fn key(&self, ip: &str) -> u64 {
        self.hasher.hash_one(ip)
    }

    fn live(&mut self, key: u64, now: Instant) -> Option<&mut Failures> {
        if self.map.get(&key).is_some_and(|f| f.since + self.window <= now) {
            self.map.remove(&key);
        }
        self.map.get_mut(&key)
    }

    /// Temps avant le prochain essai permis, zéro si permis maintenant.
    pub fn locked_for(&mut self, ip: &str, now: Instant) -> Duration {
        let key = self.key(ip);
        let (max, window) = (self.max, self.window);
        match self.live(key, now) {
            Some(f) if f.count >= max => f.since + window - now,
            _ => Duration::ZERO,
        }
    }

    pub fn record_failure(&mut self, ip: &str, now: Instant) {
        let key = self.key(ip);
        if let Some(f) = self.live(key, now) {
            f.count += 1;
            return;
        }
        if self.map.len() >= MAX_KEYS {
            self.map.pop_front();
        }
        self.map.insert(key, Failures { count: 1, since: now });
    }

    /// Un succès remet à zéro le compteur de l'IP.
    pub fn clear(&mut self, ip: &str) {
        let key = self.key(ip);
        self.map.remove(&key);
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_clear_and_eviction() {
        let mut l = Limiter::default();
        let t0 = Instant::now();
        for _ in 0..MAX_PER_IP {
            l.record_failure("1.2.3.4", t0);
        }
        assert!(l.locked_for("1.2.3.4", t0) > Duration::from_secs(14 * 60));
        assert_eq!(l.locked_for("1.2.3.5", t0), Duration::ZERO);
        assert_eq!(l.locked_for("1.2.3.4", t0 + WINDOW), Duration::ZERO);
        for _ in 0..MAX_PER_IP {
            l.record_failure("9.9.9.9", t0);
        }
        l.clear("9.9.9.9");
        assert_eq!(l.locked_for("9.9.9.9", t0), Duration::ZERO);
        for i in 0..MAX_KEYS + 10 {
            l.record_failure(&format!("ip{i}"), t0);
        }
        assert!(l.len() <= MAX_KEYS);
    }
}
