# Une image : le binaire Rust sert l'API et l'UI qu'il embarque ; llama-server
# et les bibliothèques CUDA viennent de l'image officielle llama.cpp.

# UI (Svelte) : web/dist, embarqué par le build Rust.
FROM node:22-bookworm-slim AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/ ./
RUN npm run build

# Binaire statique musl : indépendant de la glibc de l'image llama.cpp.
FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev build-base
WORKDIR /src
# Dépendances d'abord (couche en cache). build.rs exige un web/dist.
# Chaque crate du workspace : manifeste + source vide (sinon --locked échoue).
COPY Cargo.toml Cargo.lock ./
COPY crates/server/Cargo.toml crates/server/build.rs crates/server/
COPY crates/engine/Cargo.toml crates/engine/
COPY crates/hub/Cargo.toml crates/hub/
COPY crates/fake-llama/Cargo.toml crates/fake-llama/
RUN mkdir -p crates/server/src crates/engine/src crates/hub/src crates/fake-llama/src web/dist \
    && echo 'fn main() {}' > crates/server/src/main.rs && touch crates/server/src/lib.rs \
    && touch crates/engine/src/lib.rs crates/hub/src/lib.rs \
    && echo 'fn main() {}' > crates/fake-llama/src/main.rs \
    && echo '<!doctype html>' > web/dist/index.html \
    && cargo build --release --locked -p enginex-server \
    && rm -rf crates/server/src crates/engine/src crates/hub/src crates/fake-llama/src web/dist
COPY crates/ crates/
COPY --from=web /src/web/dist web/dist
# Sources plus récentes que les vides compilés ci-dessus.
RUN find crates -name '*.rs' -exec touch {} + \
    && cargo build --release --locked -p enginex-server

# Runtime : image officielle llama.cpp CUDA (llama-server dans /app).
FROM ghcr.io/ggml-org/llama.cpp:server-cuda AS runtime
ARG ENGINEX_REVISION=dev
LABEL org.opencontainers.image.source="https://github.com/R0m1k3/EngineX" \
      org.opencontainers.image.revision="${ENGINEX_REVISION}" \
      org.opencontainers.image.description="EngineX - application locale qui pilote un agent de code (llama.cpp + Codex)"
COPY --from=build /src/target/release/enginex /usr/local/bin/enginex
# Pas d'utilisateur créé : un uid n'a pas besoin d'entrée dans /etc/passwd,
# et l'image amont peut déjà avoir un uid/gid 1000.
RUN mkdir -p /data /models && chown 1000:1000 /data /models
ENV HOME=/data \
    LD_LIBRARY_PATH=/app \
    CUDA_DEVICE_ORDER=PCI_BUS_ID \
    NVIDIA_DRIVER_CAPABILITIES=compute,utility \
    ENGINEX_DATA=/data
USER 1000:1000
WORKDIR /data
EXPOSE 8090
# Remplace le HEALTHCHECK amont (qui vise llama-server sur :8080).
HEALTHCHECK --interval=30s --timeout=5s --start-period=20s --retries=3 \
    CMD curl -fsS http://127.0.0.1:8090/api/health >/dev/null || exit 1
# Remplace l'ENTRYPOINT amont (/app/llama-server).
ENTRYPOINT ["/usr/local/bin/enginex"]
CMD ["serve"]
