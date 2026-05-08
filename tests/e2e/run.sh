#!/usr/bin/env bash
# Wrapper that:
#   1. Builds the nsp:e2e image from the repo Dockerfile (skip with NO_BUILD=1).
#   2. Generates an ephemeral master key.
#   3. Brings up the e2e compose project (nsp + tester) on a private
#      bridge network `nsp-e2e-net`.
#   4. Streams logs and propagates the tester's exit code.
#   5. Tears the project down (always).
#
# Run from the repository root:
#   tests/e2e/run.sh
#
# Required: docker ≥ 24 with the compose v2 plugin, the host's
# `wireguard` kernel module loaded (the kernel backend smoke test
# needs it), and root or `docker` group membership.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
COMPOSE_FILE="$REPO_ROOT/tests/e2e/docker-compose.yml"
PROJECT="nsp-e2e"

cd "$REPO_ROOT"

if ! lsmod 2>/dev/null | grep -q '^wireguard '; then
    echo "WARNING: wireguard kernel module not loaded on host." >&2
    echo "         The kernel-backend assertion will fail." >&2
    echo "         Load it with: sudo modprobe wireguard" >&2
fi

if [[ "${NO_BUILD:-0}" != "1" ]]; then
    echo "==> building nsp:e2e from $REPO_ROOT/Dockerfile"
    DOCKER_BUILDKIT=1 docker build --progress=plain -f Dockerfile -t nsp:e2e "$REPO_ROOT"
fi

# Ephemeral 32-byte master key for this run only.
NSP_MASTER_KEY="$(openssl rand -base64 32)"
export NSP_MASTER_KEY
export NSP_ADMIN_PASSWORD="${NSP_ADMIN_PASSWORD:-changeme-e2e}"

cleanup() {
    echo "==> tearing down compose project"
    docker compose -f "$COMPOSE_FILE" -p "$PROJECT" down --remove-orphans --volumes >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "==> bringing up nsp + tester on dedicated docker network"
# `--exit-code-from tester` implies `--abort-on-container-exit` and
# returns the tester's exit code as compose's, so a failing assertion
# in run-e2e.sh cleanly propagates here.
set +e
docker compose -f "$COMPOSE_FILE" -p "$PROJECT" up \
    --build \
    --exit-code-from tester \
    tester
rc=$?
set -e

if [[ $rc -ne 0 ]]; then
    echo "==> e2e FAILED — capturing nsp logs"
    docker compose -f "$COMPOSE_FILE" -p "$PROJECT" logs nsp --tail=200 || true
fi

exit $rc
