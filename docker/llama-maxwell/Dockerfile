# llama-server patché pour Maxwell-V1-Flash (architecture `maxwell-moe`, port
# AtomicChat absent de llama.cpp officiel) : commit et patch figés (le patch
# ne s'applique que sur ce commit = b12464), vérifiés par sha256, CUDA pour la
# 3060 (86) et la 5060 Ti (120), binaire statique. Image à part, construite
# une fois par .github/workflows/llama-maxwell.yml ; l'image EngineX la copie.
FROM docker.io/nvidia/cuda:12.8.1-devel-ubuntu24.04 AS build
ARG LLAMA_COMMIT=d3c3e242c16b8cbf51400fad410cd69b22d64f35
ARG MAXWELL_REV=0e2d02c0139150c6c0f2f99cd4b6142eafa76c09
ARG MAXWELL_PATCH_SHA256=55049b8489262fc384e11c844f80a08576e71d9c6a56a7d9b226e0ead3b8d6bd
RUN apt-get update \
    && apt-get install -y --no-install-recommends build-essential cmake git curl ca-certificates \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /llama
RUN git init -q . \
    && git remote add origin https://github.com/ggml-org/llama.cpp \
    && git fetch -q --depth 1 origin "${LLAMA_COMMIT}" \
    && git checkout -q FETCH_HEAD \
    && curl -fsSL -o /tmp/maxwell.patch \
       "https://huggingface.co/AtomicChat/Maxwell-V1-Flash-GGUF/resolve/${MAXWELL_REV}/maxwell-moe-port.patch" \
    && echo "${MAXWELL_PATCH_SHA256}  /tmp/maxwell.patch" | sha256sum -c - \
    && git apply /tmp/maxwell.patch
RUN cmake -B build -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF \
       -DGGML_NATIVE=OFF -DGGML_CUDA=ON -DCMAKE_CUDA_ARCHITECTURES="86;120" \
       -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF -DLLAMA_OPENSSL=OFF \
       -DLLAMA_BUILD_NUMBER=12464 \
       -DCMAKE_EXE_LINKER_FLAGS=-Wl,--allow-shlib-undefined \
    && cmake --build build --config Release -j"$(nproc)" --target llama-server \
    && mkdir -p /opt/llama-maxwell && cp build/bin/llama-server /opt/llama-maxwell/

FROM scratch
COPY --from=build /opt/llama-maxwell /opt/llama-maxwell
