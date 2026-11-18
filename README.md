# EngineX

Application locale qui pilote un agent de code. Elle fait tourner des modèles GGUF avec llama.cpp sur GPU NVIDIA, et
Codex CLI sert d'agent dans une sandbox par session. Un seul utilisateur, tout reste sur la machine.

**État : étape 1 (socle).** Pour l'instant, EngineX fait trois choses :
- il sert l'interface, protégée par un mot de passe ;
- il affiche une page Système avec les GPU vus par `nvidia-smi`, ainsi que la version et les périphériques CUDA vus par `llama-server` ;
- il publie son image automatiquement.

Le moteur, les modèles Hugging Face, Codex, les sandboxes et la mémoire arrivent aux étapes suivantes.

## Installation (Portainer ou compose de l'Unraid)

Il faut le pilote NVIDIA et le NVIDIA Container Toolkit sur l'hôte.

1. Créez une stack à partir de `docker-compose.yml` (Portainer : *Stacks → Add stack → Repository*, en indiquant ce dépôt).
2. Démarrez-la, puis ouvrez `http://<hôte>:8090`.
3. Au premier accès, EngineX demande un **code de configuration**. Il est écrit dans les logs :
   ```sh
   docker logs enginex 2>&1 | grep "Code de configuration"
   ```
   Saisissez-le, puis choisissez le mot de passe (12 caractères minimum).

L'image `ghcr.io/r0m1k3/enginex:latest` est publiée à chaque push sur `main`, une fois les tests et le test de fumée de
l'image réussis. La mise à jour automatique de l'hôte (auto-update compose ou Watchtower : la stack porte le label
`com.centurylinklabs.watchtower.enable=true`) la redéploie. Les données restent dans le volume `enginex-data`.

> Le paquet ghcr est privé à sa création. Passez-le en public une fois (*GitHub → Packages → enginex → Package
> settings → Change visibility*), sinon il faut donner à Portainer un accès à ghcr. L'image ne contient aucun secret.

### Revenir à une version précédente

Chaque image est aussi publiée sous un tag `sha-xxxxxxx`, visible dans *Packages* sur GitHub. Remplacez `latest` par
ce tag dans la stack, puis redéployez. Remettez `latest` quand le problème est corrigé.

### Mot de passe perdu

```sh
docker exec enginex enginex reset-password
```

Le mot de passe et toutes les sessions sont effacés. Rouvrez EngineX : un nouveau code de configuration apparaît dans
`docker logs enginex`. Aucun redémarrage n'est nécessaire.

### Dossier monté au lieu du volume nommé

EngineX tourne avec l'uid 1000. Un volume nommé reprend tout seul le bon propriétaire, mais un dossier monté (par
exemple `/mnt/user/appdata/enginex:/data`) ne le fait pas. Deux solutions :
- `chown -R 1000:1000 /mnt/user/appdata/enginex` ;
- ou `user: "99:100"` (ou l'uid propriétaire du dossier) dans le service de la stack.

Si `/data` n'est pas inscriptible, le conteneur s'arrête et le message donne l'uid à utiliser.

## Accès depuis l'extérieur

EngineX ne fait pas de HTTPS. Sur le réseau local, l'accès direct suffit. Hors du réseau local, passez par un VPN
(Tailscale, WireGuard) ou par un reverse proxy HTTPS. Avec un reverse proxy :
- mettez `ENGINEX_TRUST_PROXY: "true"` dans la stack. Sinon tout le monde partage l'IP du proxy, et 5 erreurs de
  n'importe qui bloquent la connexion 15 minutes ;
- gardez l'en-tête `Host` d'origine (c'est le cas par défaut avec Caddy, Nginx Proxy Manager et Traefik) ou envoyez
  `X-Forwarded-Host`. Sinon les requêtes du navigateur sont refusées (`forbidden_origin`).

## Variables d'environnement

| Variable | Défaut | Rôle |
|---|---|---|
| `ENGINEX_DATA` | `/data` | base SQLite (`enginex.db`) |
| `ENGINEX_TRUST_PROXY` | `false` | lire `X-Forwarded-For/Proto/Host` (derrière un reverse proxy uniquement) |
| `ENGINEX_LLAMA_BIN` | `/app/llama-server` | binaire llama-server |
| `ENGINEX_NVIDIA_SMI` | `nvidia-smi` | binaire nvidia-smi |
| `ENGINEX_BIND` | `0.0.0.0:8090` | adresse d'écoute (développement ; l'image écoute sur 8090) |
| `RUST_LOG` | `info` | niveau des logs |

## Développement

Il faut Rust stable et Node 22.

```sh
cd web && npm ci && npm run dev          # UI sur http://localhost:5173, /api relayé vers :8090
ENGINEX_DATA=/tmp/ex ENGINEX_BIND=127.0.0.1:8090 \
  ENGINEX_LLAMA_BIN=$PWD/crates/server/tests/fixtures/fake-llama-server.sh \
  cargo run -p enginex-server             # API (code de configuration dans la sortie)
```

Tests et lint : voir `AGENTS.md`.

## Licence

MIT. Une partie du code vient de Loki/AJEAN et de Presencia : voir `NOTICE.md`.
