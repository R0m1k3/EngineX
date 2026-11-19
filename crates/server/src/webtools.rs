//! Outils web de Codex : `web_search` (SearXNG de la stack) et `web_read`
//! (page lue par EngineX, rendue en texte).
//!
//! `web_read` part du conteneur d'EngineX, qui voit docker-socket-proxy et le
//! réseau local : seules les adresses publiques sont permises. Chaque nom est
//! résolu une fois, toutes ses adresses contrôlées, et la connexion épinglée
//! sur l'adresse contrôlée ; les redirections sont suivies à la main avec le
//! même contrôle.

use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    time::Duration,
};

use futures_util::StreamExt;
use reqwest::{Url, header};
use serde_json::Value;

use crate::state::AppState;

const READ_TIMEOUT: Duration = Duration::from_secs(20);
const READ_MAX_BYTES: usize = 5 << 20;
const MAX_REDIRECTS: usize = 5;
pub const READ_DEFAULT_CHARS: usize = 20_000;
pub const READ_MAX_CHARS: usize = 100_000;
const SEARCH_TIMEOUT: Duration = Duration::from_secs(15);
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) EngineX/web_read";

/// Adresse joignable depuis Internet (ni privée, ni locale, ni réservée).
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => public_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return public_v4(v4);
            }
            let s = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (s[0] & 0xfe00) == 0xfc00 // unique local fc00::/7
                || (s[0] & 0xffc0) == 0xfe80 // lien-local fe80::/10
                || (s[0] == 0x2001 && s[1] == 0x0db8) // documentation
                || (s[0] == 0x0064 && s[1] == 0xff9b) // NAT64 : vise une IPv4 quelconque
                || s[0] == 0 // ::/16 (compatibles IPv4, réservées)
                || v6 == Ipv6Addr::UNSPECIFIED)
        }
    }
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let o = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || ip.is_multicast()
        || o[0] == 0
        || (o[0] == 100 && (o[1] & 0xc0) == 64) // CGNAT 100.64.0.0/10
        || (o[0] == 192 && o[1] == 0 && o[2] == 0) // 192.0.0.0/24
        || (o[0] == 198 && (o[1] & 0xfe) == 18) // bancs d'essai 198.18.0.0/15
        || o[0] >= 240)
}

/// Une adresse sûre pour `url` (première adresse résolue, toutes contrôlées).
async fn resolve(url: &Url, allow_private: bool) -> Result<SocketAddr, String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!(
            "adresse refusée : schéma {} (http ou https seulement)",
            url.scheme()
        ));
    }
    let host = url.host_str().ok_or("adresse sans hôte")?;
    let port = url.port_or_known_default().ok_or("port inconnu")?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let addrs: Vec<SocketAddr> = tokio::time::timeout(Duration::from_secs(5), tokio::net::lookup_host((host, port)))
        .await
        .map_err(|_| format!("{host} : résolution trop lente"))?
        .map_err(|e| format!("{host} : nom introuvable ({e})"))?
        .collect();
    let first = *addrs.first().ok_or_else(|| format!("{host} : aucune adresse"))?;
    if !allow_private && let Some(bad) = addrs.iter().find(|a| !is_public(a.ip())) {
        return Err(format!(
            "adresse refusée : {host} pointe vers {} (réseau privé ou local, interdit à web_read)",
            bad.ip()
        ));
    }
    Ok(first)
}

/// Page lue : adresse finale, type, texte.
pub struct Page {
    pub url: String,
    pub content_type: String,
    pub text: String,
}

/// Lit `url` et rend son texte (HTML converti, texte et JSON tels quels).
pub async fn read(state: &AppState, url: &str) -> Result<Page, String> {
    let allow_private = state.config().web.allow_private;
    let mut url = Url::parse(url.trim()).map_err(|e| format!("adresse illisible : {e}"))?;
    for _ in 0..=MAX_REDIRECTS {
        let addr = resolve(&url, allow_private).await?;
        let host = url.host_str().unwrap_or_default().to_string();
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(READ_TIMEOUT)
            .no_proxy()
            .user_agent(UA)
            .resolve(&host, addr)
            .build()
            .map_err(|e| e.to_string())?;
        let res = client
            .get(url.clone())
            .header(
                header::ACCEPT,
                "text/html,application/xhtml+xml,text/plain;q=0.9,*/*;q=0.5",
            )
            .header(header::ACCEPT_LANGUAGE, "fr,en;q=0.8")
            .send()
            .await
            .map_err(|e| format!("{url} : {}", without_url(&e)))?;
        if res.status().is_redirection() {
            let loc = res
                .headers()
                .get(header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| format!("{url} : redirection sans destination"))?;
            url = url.join(loc).map_err(|e| format!("redirection illisible : {e}"))?;
            continue;
        }
        let status = res.status();
        let ctype = res
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let kind = ctype.split(';').next().unwrap_or("").trim().to_string();
        let textual = kind.is_empty()
            || kind.starts_with("text/")
            || kind == "application/xhtml+xml"
            || kind == "application/json"
            || kind.ends_with("+json")
            || kind == "application/xml"
            || kind.ends_with("+xml");
        if !textual {
            return Err(format!("{url} : type {kind} non lisible en texte"));
        }
        let mut body = Vec::new();
        let mut stream = res.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("{url} : {}", without_url(&e)))?;
            if body.len() + chunk.len() > READ_MAX_BYTES {
                body.extend_from_slice(&chunk[..READ_MAX_BYTES - body.len()]);
                break;
            }
            body.extend_from_slice(&chunk);
        }
        let html = kind.is_empty() || kind.contains("html");
        let text = if html {
            html2text::from_read(&body[..], 100).unwrap_or_else(|_| String::from_utf8_lossy(&body).into_owned())
        } else {
            String::from_utf8_lossy(&body).into_owned()
        };
        let text = if status.is_success() {
            text
        } else {
            format!("(HTTP {})\n{text}", status.as_u16())
        };
        return Ok(Page {
            url: url.to_string(),
            content_type: kind,
            text: squeeze(&text),
        });
    }
    Err(format!("plus de {MAX_REDIRECTS} redirections"))
}

/// Message d'erreur reqwest court (l'URL est déjà donnée).
fn without_url(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "délai dépassé".into()
    } else if e.is_connect() {
        "connexion impossible".into()
    } else if e.is_body() || e.is_decode() {
        "réponse coupée".into()
    } else {
        "requête impossible".into()
    }
}

/// Lignes vides en série réduites à une.
fn squeeze(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blank = 0;
    for line in s.lines() {
        let l = line.trim_end();
        if l.is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(l);
        out.push('\n');
    }
    out.trim().to_string()
}

/// Coupe à `max` caractères en le signalant.
pub fn cut(text: &str, max: usize) -> String {
    let n = text.chars().count();
    if n <= max {
        return text.to_string();
    }
    let mut t: String = text.chars().take(max).collect();
    t.push_str(&format!("\n\n[… texte coupé : {max} caractères sur {n}]"));
    t
}

/// Un résultat de recherche.
pub struct Hit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Recherche sur le SearXNG de la stack.
pub async fn search(state: &AppState, query: &str, count: usize) -> Result<Vec<Hit>, String> {
    let base = state.config().web.searxng_url.trim_end_matches('/').to_string();
    let res = state
        .0
        .http
        .get(format!("{base}/search"))
        .query(&[("q", query), ("format", "json")])
        .timeout(SEARCH_TIMEOUT)
        .send()
        .await
        .map_err(|e| {
            tracing::warn!("SearXNG ({base}) : {e}");
            "recherche web indisponible : SearXNG ne répond pas".to_string()
        })?;
    if !res.status().is_success() {
        let code = res.status().as_u16();
        return Err(if code == 403 {
            "recherche web indisponible : SearXNG refuse le format JSON (search.formats)".into()
        } else {
            format!("recherche web indisponible : SearXNG a répondu HTTP {code}")
        });
    }
    let v: Value = res
        .json()
        .await
        .map_err(|_| "recherche web : réponse de SearXNG illisible".to_string())?;
    let hits = v["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            Some(Hit {
                title: r["title"].as_str().unwrap_or("").trim().to_string(),
                url: r["url"].as_str()?.to_string(),
                snippet: r["content"].as_str().unwrap_or("").trim().to_string(),
            })
        })
        .take(count)
        .collect();
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_addresses() {
        for bad in [
            "127.0.0.1",
            "10.1.2.3",
            "172.17.0.2",
            "192.168.1.10",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:192.168.1.1",
            "64:ff9b::a00:1",
        ] {
            assert!(!is_public(bad.parse().unwrap()), "{bad} devrait être refusée");
        }
        for ok in ["1.1.1.1", "93.184.216.34", "2606:4700:4700::1111", "::ffff:8.8.8.8"] {
            assert!(is_public(ok.parse().unwrap()), "{ok} devrait passer");
        }
    }

    #[test]
    fn cutting_and_squeezing() {
        assert_eq!(squeeze("a\n\n\n\nb  \n"), "a\n\nb");
        assert_eq!(cut("abc", 5), "abc");
        assert!(cut("abcdef", 3).starts_with("abc\n\n[… texte coupé : 3 caractères sur 6]"));
    }
}
