//! Adresse du client. `client_ip` et les plages de confiance viennent de
//! Presencia (`server/src/client_ip.rs`) ; `limiter_key` est propre à
//! EngineX.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

fn trusted_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    a == 127                               // loopback 127.0.0.0/8
        || (a == 169 && b == 254)          // linklocal 169.254.0.0/16
        || a == 10                         // uniquelocal 10.0.0.0/8
        || (a == 172 && (16..=31).contains(&b)) // 172.16.0.0/12
        || (a == 192 && b == 168) // 192.168.0.0/16
}

fn trusted_v6(ip: Ipv6Addr) -> bool {
    let s0 = ip.segments()[0];
    ip == Ipv6Addr::LOCALHOST              // ::1
        || (s0 & 0xffc0) == 0xfe80         // fe80::/10
        || (s0 & 0xfe00) == 0xfc00 // fc00::/7
}

/// Une adresse illisible n'est pas de confiance.
fn trusted(addr: &str) -> bool {
    let addr = addr.split('%').next().unwrap_or(addr);
    match addr.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => trusted_v4(v4),
        Ok(IpAddr::V6(v6)) => match v6.to_ipv4_mapped() {
            Some(v4) => trusted_v4(v4) || trusted_v6(v6),
            None => trusted_v6(v6),
        },
        Err(_) => false,
    }
}

/// Adresse du pair au format de Node (`::ffff:a.b.c.d` pour l'IPv4).
pub fn peer_string(peer: &SocketAddr) -> String {
    match peer.ip() {
        IpAddr::V4(v4) => format!("::ffff:{v4}"),
        IpAddr::V6(v6) => v6.to_string(),
    }
}

/// Le pair, puis les sauts de `X-Forwarded-For` de droite à gauche : la
/// première adresse hors confiance l'emporte, sinon le saut le plus à gauche.
pub fn client_ip(peer: &SocketAddr, forwarded_for: Option<&str>) -> String {
    let mut addrs = vec![peer_string(peer)];
    if let Some(h) = forwarded_for {
        addrs.extend(h.split(',').rev().map(|s| s.trim_matches(' ').to_string()));
    }
    let last = addrs.len() - 1;
    for (i, a) in addrs.iter().enumerate() {
        if i == last || !trusted(a) {
            return a.clone();
        }
    }
    unreachable!()
}

/// Clé du limiteur : IPv4 mappée ramenée en IPv4, IPv6 réduite à son /64
/// (un accès IPv6 domestique reçoit un /64 entier), chaîne illisible
/// gardée telle quelle.
pub fn limiter_key(ip: String) -> String {
    let bare = ip.split('%').next().unwrap_or(&ip);
    match bare.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => v4.to_string(),
        Ok(IpAddr::V6(v6)) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => {
                let s = v6.segments();
                format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
            }
        },
        Err(_) => ip,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mapped_and_ranges() {
        assert!(trusted("::ffff:10.1.2.3") && trusted("172.31.0.1") && trusted("fd00::1") && trusted("fe80::1%eth0"));
        assert!(!trusted("172.32.0.1") && !trusted("8.8.8.8") && !trusted("garbage") && !trusted(""));
    }

    #[test]
    fn keys() {
        assert_eq!(limiter_key("::ffff:10.0.0.1".into()), "10.0.0.1");
        assert_eq!(limiter_key("10.0.0.1".into()), "10.0.0.1");
        assert_eq!(
            limiter_key("2001:db8:1:2:aaaa::1".into()),
            limiter_key("2001:db8:1:2:bbbb::9".into())
        );
        assert_ne!(
            limiter_key("2001:db8:1:2::1".into()),
            limiter_key("2001:db8:1:3::1".into())
        );
        let peer: SocketAddr = "[::ffff:10.0.0.5]:1".parse().unwrap();
        assert_eq!(limiter_key(client_ip(&peer, Some("203.0.113.7"))), "203.0.113.7");
        let peer: SocketAddr = "10.0.0.5:1".parse().unwrap();
        assert_eq!(limiter_key(client_ip(&peer, None)), "10.0.0.5");
    }
}
