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
/// l'app-server, du relais et des sockets (inodes) de `skip`. Même règle que le relais.
pub fn listening(tables: &[&str], skip: &std::collections::HashSet<String>) -> Vec<u16> {
    let mut ports: Vec<u16> = tables
        .iter()
        .flat_map(|t| t.lines().skip(1))
        .filter_map(|l| {
            let f: Vec<&str> = l.split_whitespace().collect();
            if f.len() < 4 || f[3] != "0A" || f.get(9).is_some_and(|i| skip.contains(*i)) {
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

/// Inodes des sockets ouverts par les processus `codex` : pendant un tour,
/// Codex écoute sur des ports éphémères de 127.0.0.1 qui ne sont pas l'app.
pub fn codex_sockets() -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    let Ok(rd) = std::fs::read_dir("/proc") else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if !std::fs::read_to_string(p.join("comm")).is_ok_and(|c| c.trim() == "codex") {
            continue;
        }
        let Ok(fds) = std::fs::read_dir(p.join("fd")) else {
            continue;
        };
        for fd in fds.flatten() {
            if let Ok(l) = std::fs::read_link(fd.path())
                && let Some(i) = l
                    .to_str()
                    .and_then(|s| s.strip_prefix("socket:["))
                    .and_then(|s| s.strip_suffix(']'))
            {
                out.insert(i.to_string());
            }
        }
    }
    out
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
            Ok(listening(&[&a, &b], &codex_sockets()))
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
        let none = Default::default();
        assert_eq!(listening(&[t, t6], &none), vec![3000, 5173]);
        // Un socket de Codex (inode 77) n'est pas une app.
        let t = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
                 0: 0100007F:84D1 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 77\n\
                 1: 00000000:0BB8 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 78\n";
        let skip = std::iter::once("77".to_string()).collect();
        assert_eq!(listening(&[t], &skip), vec![3000]);
    }
}
