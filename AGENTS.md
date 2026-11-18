# EngineX - règles pour les agents

Application locale qui pilote un agent de code : backend Rust (`crates/server`, binaire `enginex`),
UI Svelte 5 (`web/`, embarquée dans le binaire), image Docker basée sur `ghcr.io/ggml-org/llama.cpp:server-cuda`.

## Commandes
- Tests Rust : `cargo build -p enginex-fake-llama && cargo test --workspace` (faux llama-server des tests)
- Lint : `cargo fmt --check` et `cargo clippy --workspace --all-targets -- -D warnings`
- Test de contrat avec le vrai Codex (npm, réseau) : `scripts/contract-codex.sh` (exec, app-server, pont des sessions)
- Fichiers attendus du pont (`tests/contract/appserver-*.jsonl`) : `UPDATE_FIXTURES=1 scripts/contract-codex.sh`, puis relire le diff
- Instantané de la spec des sandboxes : `UPDATE_FIXTURES=1 cargo test -p enginex-sandbox`, puis relire le diff
- Bout en bout Docker (CI, job `sandbox-e2e`) : `ENGINEX_TEST_DOCKER=1 cargo test -p enginex-server --test sandbox_docker -- --ignored`
  (réseau `enginex-sandbox-test`, proxy sur 127.0.0.1:23750 et uid 1000 : voir `.github/workflows/ci.yml`)
- Image devbox : `docker build --build-arg CODEX_VERSION=<version> docker/devbox`
- Fichiers attendus du crate `oai` : `UPDATE_FIXTURES=1 cargo test -p enginex-oai`, puis relire le diff
- Instantanés, Fichiers, vérification : `cargo test -p enginex-server --test snapshots --test files --test verify`
- UI : `cd web && npm ci && npm run check && npm run build`
- Build release (exige `web/dist`) : `cargo build --release -p enginex-server`
- Dev : `cd web && npm run dev` + `ENGINEX_DATA=/tmp/ex ENGINEX_BIND=127.0.0.1:8090 cargo run -p enginex-server`

## Règles
- Aucun secret (clé, jeton, mot de passe) dans les fichiers, les commits ou les logs.
- Toute route `/api` nouvelle passe sous `require_session`, sauf décision explicite.
- Les erreurs de l'API passent par `ApiError` (`{"error":"<code>"}`) ; les corps JSON par `ApiJson`.
  Exception : les routes `/v1/*` (Codex, clients OpenAI) répondent au format OpenAI
  `{"error":{"code","message","type"}}` par `V1Error`, et sont protégées par une clé d'API, pas par la session.
- `serde_json` sans la feature `preserve_order` (la traduction `oai` doit rester déterministe ; contrôlé en CI).
- Opérations sur le moteur : `begin_job` d'abord, porte de service (`serve_gate`) ensuite, jamais l'inverse.
- Code repris d'un autre projet : le noter dans `NOTICE.md`.
- Sandboxes : la spec du conteneur n'est construite que par `enginex_sandbox::spec::container_spec` (fonction pure,
  instantané testé) ; aucun autre montage, port ou capacité. Le proxy Docker ne donne ni `exec` ni volumes.
- Secrets du coffre : ils ne sortent que par `vault::env_for` (environnement d'une sandbox) et `vault::all_values`
  (masquage). Tout texte venant de Codex passe par le masqueur avant d'être stocké, diffusé ou envoyé au modèle.
- Notifications et requêtes de Codex : seules celles de `enginex_codex::types::WHITELIST` et `SERVER_REQUESTS` sont
  traitées ; une nouvelle méthode s'ajoute là, avec son cas dans le test de contrat.
- Fichiers d'un dossier que la sandbox écrit (topic, copie) : jamais de chemin suivi par le noyau ; passer par
  `files::open_beneath` (openat2 `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS`) et ses fonctions (`read_beneath`,
  `create_beneath`, `remove_beneath`).
- Git sur un dossier que la sandbox écrit : uniquement par le conteneur d'aide (`Backend::run_helper`, spec
  `spec::helper_spec`, sans réseau) et des scripts fixes de `snapshots.rs` (macros `prelude!` et `snap_core!`, réglages
  `-c` communs) ; aucune valeur venue de l'utilisateur dans le script, tout passe par l'environnement.
- Retours arrière, annulations et instantanés : prendre le verrou du dossier (`snapshots::lock`) une fois par fonction
  publique ; les fonctions `*_locked` le demandent en argument.
- Textes de l'UI et de la doc en français.
