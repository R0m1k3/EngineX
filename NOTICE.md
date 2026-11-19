# Avis d'attribution

EngineX reprend du code d'autres projets sous licence MIT. Merci à leurs auteurs.

| Fichier EngineX | Origine | Licence |
|---|---|---|
| `crates/engine/src/probe.rs` : `DEVICE_LINE`, `parse_list_devices`, `parse_nvidia_smi` (adaptés) | [Loki](https://github.com/R0m1k3/Loki) (`rust/loki-rs/src/backends/devices.rs`, `rust/loki-rs/src/sysinfo.rs`), fork d'[AJEAN](https://github.com/nathaninline/ajean) de nathaninline | MIT |
| `crates/browser/src/{lib,js,grid}.rs` (adaptés), `crates/browser/src/tests.rs` (en partie) | Loki, `rust/loki-rs/src/browser/` | MIT |
| `crates/server/src/web.rs`, `crates/server/build.rs` | Presencia (R0m1k3), `server/src/web.rs`, `server/build.rs` | même auteur |
| `crates/server/src/limiter.rs`, `crates/server/src/client_ip.rs` (adaptés) | Presencia, `server/src/limiter.rs`, `server/src/client_ip.rs` | même auteur |

Loki et AJEAN : Copyright (c) 2026 Jean contributors, licence MIT (texte : https://github.com/R0m1k3/Loki/blob/main/LICENSE).
