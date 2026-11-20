# Une image : le binaire Rust sert l'API et l'UI qu'il embarque ; llama-server
# et les bibliothèques CUDA viennent de l'image officielle llama.cpp.

# UI (Svelte) : web/dist, embarqué par le build Rust.
FROM node:22-bookworm-slim AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/ ./
RUN npm run build

# Squelette du workspace : manifestes + sources vides, pour la couche des
# dépendances. Généré depuis crates/* (aucune liste à tenir à jour) ; son
# contenu ne change que si un manifeste change, donc le cache tient.
FROM alpine:3 AS skeleton
WORKDIR /src
COPY crates/ crates/
RUN set -e; for d in crates/*/; do \
      mkdir -p "/out/$d/src"; cp "$d/Cargo.toml" "/out/$d"; \
      [ -f "$d/build.rs" ] && cp "$d/build.rs" "/out/$d"; \
      [ -f "$d/src/lib.rs" ] && touch "/out/$d/src/lib.rs"; \
      [ -f "$d/src/main.rs" ] && echo 'fn main() {}' > "/out/$d/src/main.rs"; \
      for b in "$d"/src/bin/*.rs; do [ -f "$b" ] || continue; \
        mkdir -p "/out/$d/src/bin"; echo 'fn main() {}' > "/out/$b"; done; \
    done; true

# Binaire statique musl : indépendant de la glibc de l'image llama.cpp.
FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev build-base
WORKDIR /src
# Dépendances d'abord (couche en cache). build.rs exige un web/dist.
COPY Cargo.toml Cargo.lock ./
COPY --from=skeleton /out/crates/ crates/
RUN mkdir -p web/dist && echo '<!doctype html>' > web/dist/index.html \
    && cargo build --release --locked -p enginex-server \
    && rm -rf crates web/dist
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
# llama-server patché pour Maxwell-V1-Flash (architecture `maxwell-moe`), image
# construite à part par .github/workflows/llama-maxwell.yml.
COPY --from=ghcr.io/r0m1k3/llama-maxwell:b12464 /opt/llama-maxwell /opt/llama-maxwell
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
