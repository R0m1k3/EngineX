#!/usr/bin/env bash
# Test de contrat : installe la dernière version de Codex CLI (npm) dans un
# dossier temporaire et lance `codex exec` contre EngineX + faux llama-server.
#   scripts/contract-codex.sh            # @openai/codex@latest
#   CODEX_VERSION=0.170.0 scripts/contract-codex.sh
set -euo pipefail
cd "$(dirname "$0")/.."
dir=$(mktemp -d)
trap 'rm -rf "$dir"' EXIT
npm install --prefix "$dir" --no-audit --no-fund --silent "@openai/codex@${CODEX_VERSION:-latest}"
codex="$dir/node_modules/.bin/codex"
echo "Codex : $("$codex" --version)"
cargo build -p enginex-fake-llama
ENGINEX_CODEX_BIN="$codex" cargo test -p enginex-server --test contract_codex -- --ignored --nocapture
