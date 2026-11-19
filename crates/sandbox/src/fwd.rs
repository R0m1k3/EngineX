//! Client du relais d'aperçu de la devbox (`docker/devbox/enginex-fwd.mjs`) :
//! joindre un port de l'app de la session, lister les ports en écoute.
//! Backend local (`fwd = None`) : connexion directe à 127.0.0.1.

use std::time::Duration;

use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

const TIMEOUT: Duration = Duration::from_secs(5);
const MAX_LINE: usize = 4096;

/// Ports TCP en écoute (état `0A`) des tables `/proc/net/tcp*`, sans ceux de
/// l'app-server et du relais. Même règle que le relais.
pub fn listening(tables: &[&str]) -> Vec<u16> {
    let mut ports: Vec<u16> = tables
        .iter()
        .flat_map(|t| t.lines().skip(1))
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() < 4 || f[3] != "0A" {
                return None;
            }
            u16::from_str_radix(f[1].rsplit(':').next()?, 16).ok()
        })
        .filter(|p| *p != 0 && *p != crate::spec::LISTEN_PORT && *p != crate::spec::FWD_PORT)
        .collect();
    ports.sort_unstable();
    ports.dedup();
    ports
}

async fn line(s: &mut TcpStream) -> anyhow::Result<String> {
    let mut out = Vec::new();
    let mut b = [0u8; 1];
    // Octet par octet : ce qui suit `OK\n` appartient à l'app.
    while out.len() < MAX_LINE {
        if s.read(&mut b).await? == 0 {
            break;
        }
        if b[0] == b'\n' {
            return Ok(String::from_utf8_lossy(&out).into_owned());
        }
        out.push(b[0]);
    }
    anyhow::bail!("réponse du relais d'aperçu illisible")
}

async fn ask(addr: &str, token: &str, cmd: &str) -> anyhow::Result<(TcpStream, String)> {
    let mut s = tokio::time::timeout(TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| anyhow::anyhow!("relais d'aperçu injoignable (délai)"))??;
    s.write_all(format!("ENGINEX-FWD {token} {cmd}\n").as_bytes()).await?;
    let reply = tokio::time::timeout(TIMEOUT, line(&mut s))
        .await
        .map_err(|_| anyhow::anyhow!("relais d'aperçu muet (délai)"))??;
    if let Some(err) = reply.strip_prefix("ERR ") {
        anyhow::bail!("{err}");
    }
    let rest = reply
        .strip_prefix("OK")
        .ok_or_else(|| anyhow::anyhow!("relais d'aperçu absent (devbox trop ancienne ?)"))?;
    Ok((s, rest.trim().to_string()))
}

/// Connexion à `port` dans la sandbox, prête à porter du HTTP.
pub async fn connect(fwd: Option<&str>, token: &str, port: u16) -> anyhow::Result<TcpStream> {
    match fwd {
        Some(addr) => Ok(ask(addr, token, &port.to_string()).await?.0),
        None => tokio::time::timeout(TIMEOUT, TcpStream::connect(("127.0.0.1", port)))
            .await
            .map_err(|_| anyhow::anyhow!("port {port} injoignable (délai)"))?
            .map_err(|_| anyhow::anyhow!("rien n'écoute sur le port {port}")),
    }
}

/// Ports en écoute dans la sandbox.
pub async fn list(fwd: Option<&str>, token: &str) -> anyhow::Result<Vec<u16>> {
    match fwd {
        Some(addr) => {
            let (_, rest) = ask(addr, token, "LIST").await?;
            Ok(serde_json::from_str(&rest)?)
        }
        None => {
            let read = |p| std::fs::read_to_string(p).unwrap_or_default();
            let (a, b) = (read("/proc/net/tcp"), read("/proc/net/tcp6"));
            Ok(listening(&[&a, &b]))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::listening;

    #[test]
    fn listening_ports() {
        let t = "  sl  local_address rem_address   st\n\
                 0: 0100007F:1435 00000000:0000 0A 0\n\
                 1: 00000000:1194 00000000:0000 0A 0\n\
                 2: 0100007F:1435 0100007F:9999 01 0\n\
                 3: 00000000:1195 00000000:0000 0A 0\n";
        let t6 = "  sl  local_address remote_address st\n\
                  0: 00000000000000000000000000000000:0BB8 00000000000000000000000000000000:0000 0A 0\n";
        assert_eq!(listening(&[t, t6]), vec![3000, 5173]);
    }
}
