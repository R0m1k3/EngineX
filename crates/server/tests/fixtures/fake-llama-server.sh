#!/bin/sh
# llama-server factice pour les tests : écrit sur la sortie d'erreur, comme le vrai.
case "$1" in
  --version)
    echo "load_backend: loaded CUDA backend" >&2
    echo "version: 7000 (abcdef0)" >&2
    echo "built with cc for x86_64-linux-gnu" >&2
    ;;
  --list-devices)
    printf 'ggml_cuda_init: found 2 CUDA devices\r\n' >&2
    printf 'Available devices:\r\n' >&2
    printf '  CUDA0: NVIDIA GeForce RTX 5060 Ti (16311 MiB, 15900 MiB free)\r\n' >&2
    printf '  bruit\r\n' >&2
    printf '  CUDA1: NVIDIA GeForce RTX 3060 (12288 MiB, 12000 MiB free)\r\n' >&2
    ;;
  *) exit 2 ;;
esac
