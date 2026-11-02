#!/usr/bin/env bash
# Set up a fresh devcontainer / Codespace so `ccx doctor` passes.
#
# Runs after the container is created (see .devcontainer/devcontainer.json).
# It installs the Rust components the workspace needs, waits for the Docker
# daemon that the docker-in-docker feature starts, and builds the pinned
# toolchain images the engine measures against.
set -euo pipefail

echo "==> devcontainer setup: CodeCurrency engine"

if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo not found; installing rustup toolchain 1.85.1"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --profile minimal --default-toolchain 1.85.1
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
fi

rustup component add rustfmt clippy || true

echo "==> waiting for the Docker daemon"
for _ in $(seq 1 60); do
    if docker info >/dev/null 2>&1; then
        break
    fi
    sleep 2
done
if ! docker info >/dev/null 2>&1; then
    echo "warning: Docker is not reachable yet; run 'scripts/pin-toolchains.sh' once it is" >&2
    exit 0
fi

echo "==> building pinned toolchain images"
scripts/pin-toolchains.sh

echo "==> building the engine"
cargo build --release

echo "==> ready: run 'target/release/ccx doctor'"
