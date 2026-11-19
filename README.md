# EngineX

Application locale qui pilote un agent de code. Elle fait tourner des modèles GGUF avec llama.cpp sur GPU NVIDIA, et
Codex CLI sert d'agent dans une sandbox par session. Un seul utilisateur, tout reste sur la machine.

**État : étape 5 (mémoire).** EngineX fait tourner les modèles GGUF : recherche et téléchargement sur Hugging Face,
presets réglés d'après la page du modèle, mise à jour de llama.cpp. Il sert une API compatible OpenAI pour Codex, et la
page Sessions permet de travailler avec Codex dans une sandbox Docker, avec instantanés, vérification automatique,
terminal et fichiers. Codex garde une mémoire en trois niveaux et retrouve les discussions passées. Les tâches, les
crons et les agents arrivent aux étapes suivantes.

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

La section **Accélération** du preset règle le décodage spéculatif et les MoE :
- *Brouillon* : aucun, **MTP** du modèle (proposé et actif d'office quand le GGUF contient les couches NextN, vérifié
  sur le tenseur `blk.N.nextn.eh_proj.weight` du dernier bloc et pas seulement sur la clé), ou un petit modèle local de
  la même architecture (`-md`, sa taille est ajoutée à la marge de `--fit`). *Jetons proposés* = `--spec-draft-n-max`.
- *Brouillon ngram* : propose la suite à partir des répétitions du contexte, sans VRAM. Retiré au lancement quand des
  experts sont en RAM (la vérification passerait par le CPU).
- *Experts MoE* : automatique (`--fit` envoie en RAM les experts qui ne tiennent pas), tous en RAM (`--cpu-moe`) ou
  les N premières couches (`--n-cpu-moe`), avec un cache GPU optionnel. Un placement manuel coupe `--fit`.
- *Lots* `-b` / `-ub` : vides = défauts de llama.cpp ; un MoE plus gros que la VRAM est créé avec 4096 / 2048.

Si llama-server s'arrête pendant le chargement avec un brouillon (contexte MTP refusé, VRAM), EngineX relance une
fois sans décodage spéculatif, le signale dans le bandeau et lance directement sans pour cette même ligne de commande
jusqu'au redémarrage. Avec deux GPU en découpe par couches, `CUDA_SCALE_LAUNCH_QUEUES=4x` est posé. Le bandeau donne
le débit de la dernière réponse et la part du brouillon acceptée, et prévient quand `--fit` a réduit le contexte :
Codex reçoit alors le contexte réellement chargé, pas celui du preset.

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

## Sessions et sandboxes

La page Sessions s'ouvre au démarrage. À gauche se trouvent les projets, leurs topics et leurs sessions ; au centre, le
fil de la session ouverte. Un topic est un dossier de travail (`/data/workspace/projects/<projet>/topics/<topic>`)
partagé par toutes ses sessions.

Chaque session a sa sandbox. Le premier message démarre un conteneur à partir de l'image
`ghcr.io/r0m1k3/enginex-devbox:latest` (Node, Python avec uv, Rust, Go, gh et Codex), et Codex y travaille dans le
dossier du topic, monté sur `/workspace`. Ouvrir ou relire une session ne démarre rien. La sandbox s'arrête après
10 minutes sans tour en cours (réglable dans Paramètres → Sessions), ou quand la session est archivée ou supprimée.
Le code et l'historique restent : au message suivant, une nouvelle sandbox démarre et Codex reprend le fil.

L'image devbox pèse plusieurs Go. Le premier message la télécharge, avec la progression affichée dans la session ;
EngineX la met ensuite à jour en fond toutes les 24 h. Quand EngineX redémarre (une mise à jour Watchtower par
exemple), il supprime les sandboxes et le tour en cours est perdu, mais le code et le fil sont gardés.

En mode d'approbation *Automatique*, Codex lance ses commandes sans demander. En mode *Prudent*, il demande votre
accord dans le fil avant chaque commande ou modification. Vous pouvez joindre, coller ou glisser des images quand le
preset actif a un fichier mmproj (vision).

### Code dans une session

Avant et après chaque tour, EngineX prend un instantané du dossier de travail dans un dépôt git à part
(`/data/snapshots/topic-<id>.git`), sans toucher au `.git` du projet. Ce git tourne dans un conteneur d'aide sans
réseau (image devbox, uid 1000), jamais dans EngineX : rien de ce que la sandbox a écrit (hooks, filtres, config d'un
dépôt imbriqué) ne s'exécute sur le serveur.

- Sous chaque tour qui a changé des fichiers, le bloc **Modifications** liste les fichiers (ajouté, modifié,
  supprimé, lignes +/-) ; un clic ouvre le diff. **Annuler ce fichier** le remet dans son état d'avant le tour ; s'il
  a encore changé depuis, EngineX demande confirmation.
- **Revenir avant ce message** (sous un de vos messages) remet les fichiers dans leur état d'avant ce message. Avec
  « Fichiers et conversation », ce message et les suivants disparaissent du fil, chez EngineX et chez Codex, et le
  texte revient dans la zone de saisie. Avec « Fichiers seulement », la conversation reste et une note prévient Codex
  au message suivant. Le bandeau **Rétablir** annule le dernier retour.
- Limites : les dossiers ignorés (`node_modules/`, `target/`, `.venv/`, `.next/`...) et les fichiers de plus de
  50 Mo ne sont pas suivis ; un dépôt git imbriqué sans commit est ignoré. Les sessions d'un même topic partagent le
  même dossier : un retour arrière attend la fin d'un tour en cours dans une autre session (`folder_busy` sinon).

### Copie séparée

Si le dossier d'un topic est un dépôt git, « + copie » à côté du topic crée une session qui travaille dans un clone
du dépôt (`/data/sessions/<id>/work`), sur la branche `enginex/<topic>-<8 caractères>`, avec le même `origin`. Les
modifications non commitées du topic ne sont pas copiées. Les instantanés, le terminal, les fichiers et la
vérification de cette session portent sur la copie. **Ramener dans le topic** commite ce qui reste dans la copie,
place la branche dans le dépôt du topic et la fusionne si le topic n'a pas de modification en cours et si la fusion
se fait sans conflit ; sinon la branche reste dans le dépôt du topic, à fusionner depuis une session du topic.
Supprimer la session supprime la copie. Les commits sont signés du nom et de l'e-mail git de Paramètres → Sessions
(un secret `GIT_AUTHOR_NAME`, `GIT_AUTHOR_EMAIL`... l'emporte dans les sandboxes).

### Vérification

Après un tour terminé qui a changé des fichiers, EngineX lance la recette de vérification dans la sandbox. Elle se
trouve dans `.enginex/verify.toml` du dossier de travail :

```toml
[[step]]
name = "tests"
run = "npm test"
timeout = 600   # secondes, 10 à 3600 (défaut 600)
```

Sans ce fichier, EngineX détecte une recette (npm, pnpm ou yarn avec `lint`, `build`, `test` ; Cargo ; Go ; pytest)
et propose de l'enregistrer. Chaque tour affiche un badge (en cours, réussie, en échec, impossible). Sur échec,
EngineX envoie lui-même le rapport à Codex, dans un message « Vérification EngineX », jusqu'à 3 tours de correction,
puis rend la main. « Revenir au dernier état vert » remet les fichiers du dernier tour vérifié. Paramètres → Sessions
règle la vérification (activée ou non, nombre de corrections) et l'identité git des instantanés.

### Terminal et fichiers

Les boutons **Fichiers** et **Terminal** du fil ouvrent un panneau à droite. Le terminal est un shell dans la
sandbox de la session (il la démarre au besoin) ; un seul par session, les secrets affichés sont masqués. Fermer le
panneau ou l'onglet termine le shell. Le panneau Fichiers montre l'arborescence du dossier de travail (sans `.git`),
l'aperçu des images, PDF, vidéos, Markdown et textes (masqués), et le téléchargement. Il est en lecture seule et ne
suit pas les liens symboliques.

### Secrets

Paramètres → Clés enregistre des secrets, pour tous les projets ou pour un seul. Chacun devient une variable
d'environnement dans les sandboxes. Les valeurs sont chiffrées avec `/data/master.key` : sauvegardez ce fichier avec le
volume, car sans lui les secrets sont illisibles. EngineX remplace les valeurs par `***` dans l'historique et dans ce
qu'il envoie au modèle. Une commande lancée dans la sandbox peut quand même les lire et les envoyer ailleurs, donc ne
donnez que des jetons aux droits limités. Les valeurs de moins de 6 caractères ne sont pas masquées. Une sandbox déjà démarrée garde ses variables jusqu'à
son arrêt : un secret ajouté, modifié ou supprimé compte au démarrage suivant.

### Accès à Docker

EngineX passe par le service `docker-proxy` de la stack (tecnativa/docker-socket-proxy) au lieu du socket Docker. Le
proxy n'autorise que les conteneurs et les images : pas d'`exec`, de volumes ni de réseaux. Il est sur un réseau
interne que les sandboxes ne voient pas. Les sandboxes ont accès à Internet et joignent EngineX par
`http://enginex:8090/v1`. Elles tournent en uid 1000, sans privilèges ni capacités, avec 8 Go de mémoire au plus.

Docker attend des chemins de l'hôte. EngineX lit donc le montage de son propre conteneur pour savoir où se trouve
`/data` sur l'hôte. Si la détection échoue, donnez ce chemin avec `ENGINEX_HOST_DATA`.

## Mémoire et skills

La page **Mémoire** montre trois niveaux : global, projet, topic. Chacun a deux fichiers :
- `instructions.md` (4 Kio au plus) : vos consignes. Codex les lit sans pouvoir les modifier ;
- `memory.md` (16 Kio au plus) : les notes de travail, une ligne datée par fait. Codex les tient pendant les sessions
  et vous pouvez les corriger.

Les fichiers sont dans `/data/workspace/memory/`, hors du dossier du topic : ils ne passent ni dans vos commits ni
dans les retours arrière, et aucune sandbox ne les voit. Au début d'une session, Codex reçoit les six fichiers de son
topic dans ses instructions. Si la mémoire change pendant la session, la nouvelle version est jointe au message
suivant. Chaque écriture garde son auteur (vous ou la session) et les 20 dernières versions de chaque fichier
restent dans l'**Historique**, d'où elles se restaurent. La suppression d'un topic ou d'un projet efface sa mémoire.

Codex écrit par les outils du serveur MCP intégré d'EngineX (`/mcp`, joint par la clé interne de la sandbox), qu'il
voit sous le nom `mcp__enginex__*` :
- `mem_read`, `mem_add` (une ligne datée, doublons refusés), `mem_replace` (un extrait unique), `mem_write`
  (réécriture complète, refusée si le fichier a changé depuis sa lecture) ;
- `recall` : recherche dans les messages de toutes les sessions (vos messages et les réponses de Codex, sans les
  tours automatiques ni les tours retirés) ;
- `session_read` : lecture d'une session trouvée par `recall`.

Au-delà de 12 Kio, Codex est invité à compacter `memory.md` ; au-delà de 16 Kio, l'ajout est refusé. Quand une
sandbox arrive au bout de son délai d'inactivité après au moins un de vos messages, EngineX lance d'abord un tour
« Mise à jour de la mémoire en fin de session » (10 minutes au plus, replié dans le fil, sans commande ni
modification de fichier), puis arrête la sandbox. Votre message suivant interrompt ce tour. Il se désactive dans
Paramètres → Sessions.

La recherche de la liste des sessions utilise le même index : elle trouve les débuts de mots (« vérif » trouve
« vérification ») et ignore les accents, mais ne trouve plus un morceau au milieu d'un mot.

Paramètres → **Skills** liste les skills de Codex (`/data/workspace/skills/<nom>/SKILL.md`). `memory` et
`memory-compaction` sont livrées ; elles ne sont jamais écrasées une fois modifiées. Une skill se crée, se modifie et
se désactive depuis cette page ; elle est copiée dans chaque sandbox à son démarrage.

Une note écrite dans la mémoire passe dans toutes les sessions suivantes du niveau. Codex a pour consigne de ne pas
y recopier de consigne lue dans un fichier ou une page web, mais relisez l'historique si une session a lu du contenu
douteux.

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
| `ENGINEX_DATA` | `/data` | base SQLite (`enginex.db`), clé du coffre (`master.key`), dossiers des topics et des sessions |
| `ENGINEX_TRUST_PROXY` | `false` | lire `X-Forwarded-For/Proto/Host` (derrière un reverse proxy uniquement) |
| `ENGINEX_LLAMA_BIN` | absent | impose le binaire llama-server (tests, dépannage) ; absent : moteur choisi dans Paramètres → Moteur, sinon `/app/llama-server` |
| `ENGINEX_MODELS` | `/models` | dossier des modèles GGUF |
| `ENGINEX_LLAMA_PORT` | `8081` | port local de llama-server (127.0.0.1) |
| `ENGINEX_NVIDIA_SMI` | `nvidia-smi` | binaire nvidia-smi |
| `ENGINEX_IMAGE_LLAMA_BIN` | `/app/llama-server` | moteur de l'image (recours) |
| `ENGINEX_HF_BASE` | `https://huggingface.co` | adresse de Hugging Face (tests) |
| `ENGINEX_OCI_REGISTRY` | `https://ghcr.io` | registre des images llama.cpp (tests) |
| `ENGINEX_BIND` | `0.0.0.0:8090` | adresse d'écoute (développement ; l'image écoute sur 8090) |
| `ENGINEX_SANDBOX` | `docker` | `docker`, ou `local` (développement : Codex lancé en processus, sans isolation) |
| `ENGINEX_DOCKER_HOST` | `tcp://docker-proxy:2375` | accès à Docker (le proxy de la stack) |
| `ENGINEX_DEVBOX_IMAGE` | `ghcr.io/r0m1k3/enginex-devbox:latest` | image des sandboxes |
| `ENGINEX_SANDBOX_NETWORK` | `enginex-sandbox` | réseau Docker des sandboxes |
| `ENGINEX_SANDBOX_API_URL` | `http://enginex:8090/v1` | adresse d'EngineX vue depuis une sandbox |
| `ENGINEX_SANDBOX_MEMORY` | `8g` | mémoire maximale d'une sandbox |
| `ENGINEX_HOST_DATA` | absent | chemin de `/data` sur l'hôte, si la détection échoue |
| `ENGINEX_CODEX_BIN` | `codex` | binaire Codex du mode `local` |
| `RUST_LOG` | `info` | niveau des logs |

## Développement

Il faut Rust stable et Node 22.

```sh
cd web && npm ci && npm run dev          # UI sur http://localhost:5173, /api relayé vers :8090
cargo build -p enginex-fake-llama        # faux llama-server (pas de GPU nécessaire)
ENGINEX_DATA=/tmp/ex/data ENGINEX_MODELS=/tmp/ex/models ENGINEX_BIND=127.0.0.1:8090 \
  ENGINEX_IMAGE_LLAMA_BIN=$PWD/target/debug/fake-llama-server \
  ENGINEX_SANDBOX=local ENGINEX_CODEX_BIN=$(which codex) \
  ENGINEX_SANDBOX_API_URL=http://127.0.0.1:8090/v1 \
  cargo run -p enginex-server             # API (code de configuration dans la sortie)
```

Tests et lint : voir `AGENTS.md`. Test de contrat avec le vrai Codex : `scripts/contract-codex.sh`.

## Licence

MIT. Une partie du code vient de Loki/AJEAN et de Presencia : voir `NOTICE.md`.
