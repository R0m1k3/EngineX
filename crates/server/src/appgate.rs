//! Sas d'app : le navigateur piloté (`browser_*`) ouvre l'app que l'agent a
//! lancée dans la sandbox de sa session.
//!
//! Le Chromium de la stack ne joint pas la sandbox. Pour chaque (session,
//! port), EngineX ouvre un port d'écoute sur son adresse du réseau du
//! navigateur (l'adresse locale d'une connexion vers lui), n'accepte que
//! l'IP du navigateur et relaie vers l'app par le relais d'aperçu
//! (`preview::forward`). Pas de cookie : deux sessions ne se mélangent pas.
//! Le modèle écrit et lit `http://localhost:<port>` ; l'adresse du sas est
//! réécrite dans les réponses.

use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    net::SocketAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

use axum::{
    Router,
    extract::{ConnectInfo, Request},
    http::StatusCode,
};
use tokio::net::{TcpListener, TcpStream};

use crate::{bridge, preview, state::AppState};

/// Sas gardés par session (le plus vieux part d'abord).
const MAX_PER_SESSION: usize = 8;

struct Gate {
    origin: String,
    task: tokio::task::JoinHandle<()>,
    created: Instant,
}

#[derive(Default)]
pub struct Gates {
    map: Mutex<HashMap<(String, u16), Gate>>,
    /// Empreinte du dernier texte de page rendu au modèle, par session.
    texts: Mutex<HashMap<String, u64>>,
}

impl Gates {
    fn origin(&self, sid: &str, port: u16) -> Option<String> {
        let m = self.map.lock().expect("verrou");
        m.get(&(sid.to_string(), port)).map(|g| g.origin.clone())
    }

    /// La session a-t-elle un sas ouvert.
    pub fn has(&self, sid: &str) -> bool {
        self.map.lock().expect("verrou").keys().any(|(s, _)| s == sid)
    }

    /// Port de l'app derrière l'origine `origin` (`location.origin` de
    /// l'onglet), si c'est un sas de la session.
    pub fn port_of(&self, sid: &str, origin: &str) -> Option<u16> {
        let m = self.map.lock().expect("verrou");
        m.iter()
            .find(|((s, _), g)| s == sid && g.origin == origin)
            .map(|((_, p), _)| *p)
    }

    /// Remplace l'adresse des sas de la session par `http://localhost:<port>`.
    pub fn rewrite(&self, sid: &str, text: &str) -> String {
        let mut pairs: Vec<(String, u16)> = {
            let m = self.map.lock().expect("verrou");
            m.iter()
                .filter(|((s, _), _)| s == sid)
                .map(|((_, p), g)| (g.origin.clone(), *p))
                .collect()
        };
        // La plus longue d'abord : `…:4123` est un préfixe de `…:41234`.
        pairs.sort_by_key(|(o, _)| std::cmp::Reverse(o.len()));
        let mut out = text.to_string();
        for (origin, port) in pairs {
            if out.contains(&origin) {
                out = out.replace(&origin, &format!("http://localhost:{port}"));
            }
        }
        out
    }

    /// `true` si le texte de la page diffère du dernier rendu à la session.
    pub fn text_changed(&self, sid: &str, text: &str) -> bool {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut h);
        let v = h.finish();
        self.texts.lock().expect("verrou").insert(sid.to_string(), v) != Some(v)
    }

    /// Ferme les sas de la session (sandbox arrêtée, session supprimée).
    pub fn close_session(&self, sid: &str) {
        let mut m = self.map.lock().expect("verrou");
        m.retain(|(s, _), g| {
            if s == sid {
                g.task.abort();
            }
            s != sid
        });
        drop(m);
        self.texts.lock().expect("verrou").remove(sid);
    }

    fn insert(&self, sid: &str, port: u16, gate: Gate) -> String {
        let mut m = self.map.lock().expect("verrou");
        let key = (sid.to_string(), port);
        // Deux ouvertures en même temps : la première gagne.
        if let Some(g) = m.get(&key) {
            gate.task.abort();
            return g.origin.clone();
        }
        while m.keys().filter(|(s, _)| s == sid).count() >= MAX_PER_SESSION {
            let oldest = m
                .iter()
                .filter(|((s, _), _)| s == sid)
                .min_by_key(|(_, g)| g.created)
                .map(|(k, _)| k.clone());
            match oldest.and_then(|k| m.remove(&k)) {
                Some(g) => g.task.abort(),
                None => break,
            }
        }
        let origin = gate.origin.clone();
        m.insert(key, gate);
        origin
    }
}

/// Une adresse de la sandbox écrite par le modèle : `http://localhost:3000/x`,
/// `localhost:5173`, `127.0.0.1:8000/a?b=1`, `0.0.0.0:8080`, `[::1]:3000`.
/// Rend le port de l'app (80 sans port) et le chemin (avec sa requête).
pub fn local_target(url: &str) -> Option<(u16, String)> {
    let u = url.trim();
    let low = u.to_ascii_lowercase();
    let rest = if low.starts_with("http://") {
        &u[7..]
    } else if low.starts_with("https://") {
        &u[8..]
    } else if low.contains("://") || low.starts_with("data:") || low.starts_with("about:") {
        return None;
    } else {
        u
    };
    let cut = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(cut);
    let (host, port) = if let Some(end) = authority.find(']') {
        let p = authority[end + 1..].strip_prefix(':');
        (&authority[..=end], p)
    } else {
        match authority.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (authority, None),
        }
    };
    let host = host.to_ascii_lowercase();
    let local =
        matches!(host.as_str(), "localhost" | "127.0.0.1" | "0.0.0.0" | "[::1]") || host.ends_with(".localhost");
    if !local {
        return None;
    }
    let port = match port {
        Some(p) => p.parse::<u16>().ok().filter(|p| *p != 0)?,
        None => 80,
    };
    let path = match tail.chars().next() {
        Some('/') => tail.to_string(),
        Some(_) => format!("/{tail}"),
        None => "/".to_string(),
    };
    Some((port, path))
}

/// Ouvre (ou retrouve) le sas vers l'app qui écoute sur `port` dans la
/// sandbox de la session. Rend son origine, vue du navigateur. Erreur
/// lisible par le modèle si la sandbox est arrêtée ou si rien n'écoute.
pub async fn open_app(state: &AppState, sid: &str, port: u16) -> Result<String, String> {
    let hub = bridge::hub(state, sid);
    let Some((fwd, token)) = hub.preview_target() else {
        return Err("la sandbox de la session est arrêtée : rien ne tourne sur localhost".into());
    };
    if enginex_sandbox::fwd::connect(fwd.as_deref(), &token, port)
        .await
        .is_err()
    {
        let ports = enginex_sandbox::fwd::list(fwd.as_deref(), &token)
            .await
            .unwrap_or_default();
        let open = if ports.is_empty() {
            "aucun port n'est ouvert".to_string()
        } else {
            let l: Vec<String> = ports.iter().map(u16::to_string).collect();
            format!("ports ouverts : {}", l.join(", "))
        };
        return Err(format!(
            "rien n'écoute sur le port {port} de la sandbox ({open}). Lance d'abord l'app en tâche de fond \
             (par exemple `npm run dev`, ou `python3 -m http.server {port}` pour des fichiers statiques), \
             attends qu'elle affiche son adresse, puis rouvre http://localhost:<port>."
        ));
    }
    if let Some(o) = state.0.gates.origin(sid, port) {
        return Ok(o);
    }
    let url = state
        .0
        .browser
        .url()
        .ok_or("navigateur non configuré (ENGINEX_BROWSER_URL)")?;
    let ep = enginex_browser::resolve(url).await?;
    let browser: SocketAddr = ep
        .trim_start_matches("http://")
        .parse()
        .map_err(|e| format!("adresse du navigateur illisible ({ep}) : {e}"))?;
    // L'adresse locale de la connexion vers le navigateur est celle par
    // laquelle il joint EngineX (même réseau de la stack).
    let probe = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(browser))
        .await
        .map_err(|_| "navigateur injoignable (délai)".to_string())?
        .map_err(|e| format!("navigateur injoignable : {e}"))?;
    let me = probe.local_addr().map_err(|e| format!("adresse locale : {e}"))?.ip();
    drop(probe);
    let listener = TcpListener::bind(SocketAddr::new(me, 0))
        .await
        .map_err(|e| format!("sas d'app : écoute impossible sur {me} : {e}"))?;
    let addr = listener.local_addr().map_err(|e| format!("sas d'app : {e}"))?;
    let allowed = browser.ip().to_canonical();
    let (st, s) = (state.clone(), sid.to_string());
    let router = Router::new().fallback(move |ConnectInfo(peer): ConnectInfo<SocketAddr>, req: Request| {
        let (st, s) = (st.clone(), s.clone());
        async move {
            if peer.ip().to_canonical() != allowed {
                return preview::page(StatusCode::FORBIDDEN, "Réservé au navigateur d'EngineX.");
            }
            preview::forward(&st, &s, port, req).await
        }
    });
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>()).await;
    });
    Ok(state.0.gates.insert(
        sid,
        port,
        Gate {
            origin: format!("http://{addr}"),
            task,
            created: Instant::now(),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_addresses() {
        let t = |u: &str| local_target(u);
        assert_eq!(t("http://localhost:3000"), Some((3000, "/".into())));
        assert_eq!(t("localhost:5173/todo?x=1#a"), Some((5173, "/todo?x=1#a".into())));
        assert_eq!(t("HTTP://127.0.0.1:8000/a/b"), Some((8000, "/a/b".into())));
        assert_eq!(t("https://0.0.0.0:8080?x=1"), Some((8080, "/?x=1".into())));
        assert_eq!(t("http://[::1]:3000/x"), Some((3000, "/x".into())));
        assert_eq!(t("http://app.localhost:4000/"), Some((4000, "/".into())));
        assert_eq!(t("localhost"), Some((80, "/".into())));
        assert_eq!(t("http://localhost:0/"), None);
        assert_eq!(t("http://localhost:abc/"), None);
        assert_eq!(t("https://example.com/localhost:3000"), None);
        assert_eq!(t("example.com:3000"), None);
        assert_eq!(t("http://localhost.example.com:3000"), None);
        assert_eq!(t("data:text/html,localhost:3000"), None);
        assert_eq!(t("back"), None);
        assert_eq!(t("file://localhost/etc/passwd"), None);
    }

    #[tokio::test]
    async fn rewrite_and_lookup() {
        let g = Gates::default();
        let gate = |origin: &str| Gate {
            origin: origin.into(),
            task: tokio::spawn(async {}),
            created: Instant::now(),
        };
        g.insert("s1", 4123, gate("http://172.20.0.3:4123"));
        g.insert("s1", 5173, gate("http://172.20.0.3:41234"));
        g.insert("s2", 5173, gate("http://172.20.0.3:50000"));
        assert_eq!(
            g.rewrite(
                "s1",
                "# App\nhttp://172.20.0.3:41234/todo\nvoir http://172.20.0.3:4123/x et http://172.20.0.3:50000/"
            ),
            "# App\nhttp://localhost:5173/todo\nvoir http://localhost:4123/x et http://172.20.0.3:50000/"
        );
        assert_eq!(g.port_of("s1", "http://172.20.0.3:41234"), Some(5173));
        assert_eq!(g.port_of("s2", "http://172.20.0.3:41234"), None);
        // Même (session, port) : le premier sas reste.
        assert_eq!(
            g.insert("s1", 5173, gate("http://172.20.0.3:9")),
            "http://172.20.0.3:41234"
        );
        assert!(g.text_changed("s1", "a"));
        assert!(!g.text_changed("s1", "a"));
        assert!(g.text_changed("s1", "b"));
        g.close_session("s1");
        assert!(!g.has("s1") && g.has("s2"));
        assert!(g.text_changed("s1", "b"));
    }

    #[tokio::test]
    async fn oldest_gate_leaves_first() {
        let g = Gates::default();
        for p in 0..(MAX_PER_SESSION as u16 + 2) {
            g.insert(
                "s",
                3000 + p,
                Gate {
                    origin: format!("http://10.0.0.1:{}", 40000 + p),
                    task: tokio::spawn(async {}),
                    created: Instant::now() + Duration::from_millis(u64::from(p)),
                },
            );
        }
        assert_eq!(g.port_of("s", "http://10.0.0.1:40000"), None);
        assert_eq!(g.port_of("s", "http://10.0.0.1:40001"), None);
        assert_eq!(g.port_of("s", "http://10.0.0.1:40002"), Some(3002));
    }
}
