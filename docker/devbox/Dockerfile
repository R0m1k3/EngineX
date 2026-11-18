# Devbox : image des sandboxes de session (un conteneur par session).
# Codex app-server y tourne en uid 1000 ; le topic est monté sur /workspace.
# CODEX_VERSION vient du job `contract` de la CI (version testée contre EngineX).
FROM node:22-bookworm

ARG CODEX_VERSION=latest

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       git curl ca-certificates ripgrep jq python3 python3-venv build-essential \
       pkg-config libssl-dev unzip \
    && mkdir -p -m 755 /etc/apt/keyrings \
    && curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
       -o /etc/apt/keyrings/githubcli-archive-keyring.gpg \
    && chmod go+r /etc/apt/keyrings/githubcli-archive-keyring.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
       > /etc/apt/sources.list.d/github-cli.list \
    && apt-get update && apt-get install -y --no-install-recommends gh \
    && rm -rf /var/lib/apt/lists/*

# uv (script officiel), binaires dans /usr/local/bin.
RUN curl -LsSf https://astral.sh/uv/install.sh | env UV_INSTALL_DIR=/usr/local/bin UV_NO_MODIFY_PATH=1 sh

# Rust stable : rustup et la chaîne dans /usr/local ; CARGO_HOME pointe
# ensuite vers le cache du projet (/cache/cargo).
ENV RUSTUP_HOME=/usr/local/rustup
RUN curl -fsSL https://sh.rustup.rs | env CARGO_HOME=/usr/local/cargo sh -s -- -y --no-modify-path \
       --profile minimal --component clippy,rustfmt --default-toolchain stable \
    && chmod -R a+rX /usr/local/rustup /usr/local/cargo

# Go : dernière version stable (archive officielle).
RUN v=$(curl -fsSL 'https://go.dev/VERSION?m=text' | head -n1) \
    && arch=$(dpkg --print-architecture) \
    && curl -fsSL "https://go.dev/dl/${v}.linux-${arch}.tar.gz" | tar -C /usr/local -xz
ENV PATH=/usr/local/go/bin:$PATH

# Codex CLI.
RUN npm i -g --no-audit --no-fund "@openai/codex@${CODEX_VERSION}" && npm cache clean --force \
    && codex --version

# Utilisateur dev 1000:1000 (l'image node a déjà `node` en 1000 : renommé).
RUN usermod -l dev -d /home/dev -m node && groupmod -n dev node \
    && mkdir -p /workspace /cache /home/dev/.codex && chown 1000:1000 /workspace /cache /home/dev/.codex

ENV HOME=/home/dev \
    CODEX_HOME=/home/dev/.codex \
    npm_config_cache=/cache/npm \
    UV_CACHE_DIR=/cache/uv \
    CARGO_HOME=/cache/cargo \
    GOMODCACHE=/cache/go/mod \
    GOCACHE=/cache/go/build \
    PATH=/cache/cargo/bin:/usr/local/go/bin:/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

USER 1000:1000
WORKDIR /workspace
CMD ["codex", "app-server", "--listen", "ws://0.0.0.0:4500", "--ws-auth", "capability-token", "--ws-token-file", "/run/enginex/token"]
