set shell := ["bash", "-cu"]

# List available recipes.
default:
    @just --list

# Run the local backend and frontend together.
dev:
    #!/usr/bin/env bash
    set -euo pipefail
    just dev-backend &
    backend_pid=$!
    just dev-frontend &
    frontend_pid=$!
    trap 'kill "$backend_pid" "$frontend_pid" 2>/dev/null || true' INT TERM EXIT
    wait -n "$backend_pid" "$frontend_pid"

# Run the backend from the local debug build, registered at nsl:/api.
# Shadowsocks is on by default (userspace proxy, no privileges needed).
# WireGuard stays off — it needs a TUN device + CAP_NET_ADMIN, which the
# Docker image provides but `cargo run` from a shell does not.
dev-backend:
    #!/usr/bin/env bash
    set -euo pipefail
    exec nsl run --force -n nsp:/api -- cargo run -p nsp -- serve \
        --listen "127.0.0.1:NSL_PORT" \
        --domain "${NSP_DOMAIN:-nsp.localhost}" \
        --tls-enabled "${NSP_TLS:-false}" \
        --storage-db-path "${NSP_DB:-data/nsp-dev.sqlite}" \
        --storage-work-dir "${NSP_WORK_DIR:-data/nsp-work}" \
        --allow-insecure-no-master-key "${NSP_ALLOW_INSECURE_NO_MASTER_KEY:-true}" \
        --security-admin-password "${NSP_ADMIN_PASSWORD:-changeme}" \
        --wireguard-enabled "${NSP_WG:-false}" \
        --shadowsocks-enabled "${NSP_SS:-true}" \
        --backup-enabled "${NSP_BACKUP:-false}" \
        --logging-level "${NSP_LOG:-info}"

# Run the Vite frontend dev server. The ui `dev` script itself registers
# vite at the nsl root (nsp), so backend `/api` and UI share one origin.
dev-frontend:
    cd ui && bun run dev

# Format Rust code.
fmt:
    cargo fmt --all

# Check Rust formatting.
fmt-check:
    cargo fmt --all -- --check

# Check Rust workspace compilation.
check:
    cargo check --workspace --all-targets

# Run strict Rust lints.
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Run Rust tests.
test:
    cargo test --workspace --all-features

# Run Rust quality gates.
rust-verify: fmt-check check clippy test

# Run UI type checking.
ui-typecheck:
    cd ui && bun run typecheck

# Run UI tests.
ui-test:
    cd ui && bun run test

# Build the UI.
ui-build:
    cd ui && bun run build

# Run UI lint and formatting checks.
ui-lint:
    cd ui && bun run lint

# Run UI quality gates.
ui-verify: ui-typecheck ui-test ui-build ui-lint

# Run all local quality gates.
verify: rust-verify ui-verify

# Build the debug binary.
build:
    cargo build -p nsp

# Build the release binary.
build-release:
    cargo build --release -p nsp

# Generate a base64 master key.
generate-key:
    cargo run -p nsp -- generate-key

# Build the release Docker image used by e2e smoke tests.
docker-build: ui-build
    docker build -f Dockerfile.release -t nsp:e2e .

# Run a container smoke test against /api/healthz.
e2e: docker-build
    #!/usr/bin/env bash
    set -euo pipefail
    name="${NSP_E2E_CONTAINER:-nsp-e2e}"
    port="${NSP_E2E_PORT:-18080}"
    docker rm -f "$name" >/dev/null 2>&1 || true
    trap 'docker rm -f "$name" >/dev/null 2>&1 || true' EXIT
    docker run -d \
        --name "$name" \
        -p "127.0.0.1:${port}:8080" \
        -e NSP_LISTEN=0.0.0.0:8080 \
        -e NSP_TLS=false \
        -e NSP_ALLOW_INSECURE_NO_MASTER_KEY=true \
        -e NSP_ADMIN_PASSWORD=changeme \
        -e NSP_DB=/work/data/e2e.sqlite \
        -e NSP_WORK_DIR=/work \
        nsp:e2e >/dev/null
    for _ in {1..30}; do
        if curl -fsS "http://127.0.0.1:${port}/api/healthz" >/dev/null; then
            exit 0
        fi
        sleep 1
    done
    docker logs "$name"
    exit 1

# Remove repo-local development state.
clean-dev:
    rm -rf data/nsp-dev.sqlite data/nsp-dev.sqlite-shm data/nsp-dev.sqlite-wal data/nsp-work
