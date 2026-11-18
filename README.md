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

## Modèles et moteur

**Modèles → Chercher** interroge Hugging Face (dépôts GGUF). La fiche d'un dépôt regroupe les fichiers par
quantification, avec une estimation de la place en VRAM sur les deux cartes au contexte et au type de KV choisis
(*tient*, *juste*, *ne tient pas*). C'est une estimation prudente : les modèles à fenêtre glissante (Gemma 3...) sont
surestimés. **Télécharger** met le fichier dans la file (un à la fois, reprise après coupure, sha256 vérifié) ; les
modèles vont dans `/models/<auteur>/<dépôt>/`.

À la fin d'un téléchargement, EngineX crée un **preset** : contexte, KV `q8_0`, flash attention, placement automatique
`--fit` sur les deux GPU, et les réglages d'échantillonnage lus sur la page Hugging Face du dépôt, puis sur celle du
modèle de base, puis dans `generation_config.json`, puis dans le GGUF. La fiche du preset montre d'où vient chaque
valeur (avec l'extrait de la page) et propose un choix quand la page en donne plusieurs. L'échantillonnage s'applique
à chaque requête sans recharger le modèle ; changer le contexte, le KV ou le placement recharge le modèle.

**Activer** lance llama-server (un seul à la fois, sur `127.0.0.1:8081` dans le conteneur). Le bandeau du haut donne
l'état, la progression du chargement, la VRAM par carte et les logs.

**Paramètres → Moteur** gère llama.cpp : *Vérifier* lit la dernière version publiée (`ghcr.io/ggml-org/llama.cpp`,
tag `server-cuda`), *Installer* la télécharge dans `/data/engines/`, la teste (version, 2 cartes vues, `--fit`) puis
recharge le preset actif avec elle ; si le modèle ne charge pas, EngineX revient tout seul à la version d'avant.
*Revenir à la précédente* échange la version active et la précédente. Le moteur fourni par l'image reste toujours
disponible en recours.

Sur l'Unraid, montez les modèles sur un disque plutôt que dans le volume Docker :
`/mnt/user/appdata/enginex/models:/models` dans la stack, puis `chown -R 1000:1000` sur ce dossier.

## Utiliser Codex avec EngineX

EngineX a une API compatible OpenAI sur `/v1` : `/v1/responses` (le protocole de Codex), `/v1/chat/completions` et
`/v1/models`. Elle sert le preset actif, et l'échantillonnage du preset est ajouté à chaque requête.

1. **Paramètres → API → Nouvelle clé** : donnez un nom (`codex-portable`) et copiez la clé, qui ne sera plus affichée.
2. Copiez le `config.toml` proposé dans `~/.codex/config.toml`, et la ligne `export ENGINEX_API_KEY=...` dans votre shell.
3. `codex` ou `codex exec "..."` utilise alors le modèle chargé par EngineX.

`/v1` demande la clé et refuse le cookie de l'UI. Une clé révoquée est refusée tout de suite. Après 20 clés refusées
depuis une même adresse, `/v1` est bloqué 15 minutes pour cette adresse ; la connexion à l'UI reste ouverte. Pendant un
chargement (activation, changement de version), les requêtes attendent qu'il finisse, et un chargement attend la fin
des requêtes en cours.

**Mise à jour automatique de llama.cpp** (Paramètres → Moteur, activée par défaut) : toutes les 6 heures, EngineX
regarde si une version plus récente est publiée. Il la télécharge et la teste sans toucher au modèle en service, puis
la met en service quand le modèle est au repos (10 minutes sans requête, aucun téléchargement en cours). Si le modèle
ne charge pas ou si les appels d'outils ne passent plus, EngineX revient à la version d'avant et ne retente plus
celle-là ; l'installer à la main la remet en jeu.

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
| `ENGINEX_LLAMA_BIN` | absent | impose le binaire llama-server (tests, dépannage) ; absent : moteur choisi dans Paramètres → Moteur, sinon `/app/llama-server` |
| `ENGINEX_MODELS` | `/models` | dossier des modèles GGUF |
| `ENGINEX_LLAMA_PORT` | `8081` | port local de llama-server (127.0.0.1) |
| `ENGINEX_NVIDIA_SMI` | `nvidia-smi` | binaire nvidia-smi |
| `ENGINEX_IMAGE_LLAMA_BIN` | `/app/llama-server` | moteur de l'image (recours) |
| `ENGINEX_HF_BASE` | `https://huggingface.co` | adresse de Hugging Face (tests) |
| `ENGINEX_OCI_REGISTRY` | `https://ghcr.io` | registre des images llama.cpp (tests) |
| `ENGINEX_BIND` | `0.0.0.0:8090` | adresse d'écoute (développement ; l'image écoute sur 8090) |
| `RUST_LOG` | `info` | niveau des logs |

## Développement

Il faut Rust stable et Node 22.

```sh
cd web && npm ci && npm run dev          # UI sur http://localhost:5173, /api relayé vers :8090
cargo build -p enginex-fake-llama        # faux llama-server (pas de GPU nécessaire)
ENGINEX_DATA=/tmp/ex/data ENGINEX_MODELS=/tmp/ex/models ENGINEX_BIND=127.0.0.1:8090 \
  ENGINEX_IMAGE_LLAMA_BIN=$PWD/target/debug/fake-llama-server \
  cargo run -p enginex-server             # API (code de configuration dans la sortie)
```

Tests et lint : voir `AGENTS.md`. Test de contrat avec le vrai Codex : `scripts/contract-codex.sh`.

## Licence

MIT. Une partie du code vient de Loki/AJEAN et de Presencia : voir `NOTICE.md`.
