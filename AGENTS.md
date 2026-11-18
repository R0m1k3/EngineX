# EngineX - règles pour les agents

Application locale qui pilote un agent de code : backend Rust (`crates/server`, binaire `enginex`),
UI Svelte 5 (`web/`, embarquée dans le binaire), image Docker basée sur `ghcr.io/ggml-org/llama.cpp:server-cuda`.

## Commandes
- Tests Rust : `cargo build -p enginex-fake-llama && cargo test --workspace` (faux llama-server des tests)
- Lint : `cargo fmt --check` et `cargo clippy --workspace --all-targets -- -D warnings`
- UI : `cd web && npm ci && npm run check && npm run build`
- Build release (exige `web/dist`) : `cargo build --release -p enginex-server`
- Dev : `cd web && npm run dev` + `ENGINEX_DATA=/tmp/ex ENGINEX_BIND=127.0.0.1:8090 cargo run -p enginex-server`

## Règles
- Aucun secret (clé, jeton, mot de passe) dans les fichiers, les commits ou les logs.
- Toute route `/api` nouvelle passe sous `require_session`, sauf décision explicite.
- Les erreurs de l'API passent par `ApiError` (`{"error":"<code>"}`) ; les corps JSON par `ApiJson`.
- Code repris d'un autre projet : le noter dans `NOTICE.md`.
- Textes de l'UI et de la doc en français.
