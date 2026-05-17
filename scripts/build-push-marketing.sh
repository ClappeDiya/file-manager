#!/usr/bin/env bash
# =============================================================================
# Build FileManager marketing image locally and push to GHCR.
#
# Usage:   ./scripts/build-push-marketing.sh [tag]
# Default: pushes both sha-<short> (immutable) and `production` (moving) tags.
# Auth:    uses `gh auth token` (run once: gh auth login --scopes write:packages)
# =============================================================================
set -euo pipefail

ENV_TAG="${1:-production}"
REGISTRY="ghcr.io/clappediya/filemanager-marketing"
PLATFORM="${BUILD_PLATFORM:-linux/amd64}"

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

VERSION="$(git rev-parse --short HEAD)"
FULL_SHA="$(git rev-parse HEAD)"

echo "============================================================"
echo " Build + push filemanager-marketing"
echo " Tag:      sha-${VERSION} + ${ENV_TAG}"
echo " Registry: ${REGISTRY}"
echo " Platform: ${PLATFORM}"
echo " Commit:   ${FULL_SHA}"
echo "============================================================"

command -v gh >/dev/null 2>&1 || {
    echo "install gh CLI: https://cli.github.com/" >&2; exit 1;
}

GH_TOKEN_VALUE="$(gh auth token 2>/dev/null || true)"
[[ -n "${GH_TOKEN_VALUE}" ]] || {
    echo "Run: gh auth login --scopes write:packages" >&2; exit 1;
}
GH_USER="$(gh api user --jq .login 2>/dev/null)"

echo "Logging into ghcr.io as ${GH_USER}..."
echo "${GH_TOKEN_VALUE}" | docker login ghcr.io -u "${GH_USER}" --password-stdin >/dev/null

# Dedicated buildx builder (QEMU/Rosetta cross-compile from arm64 macs to amd64)
if ! docker buildx inspect filemanager-builder >/dev/null 2>&1; then
    echo "Creating buildx builder filemanager-builder..."
    docker buildx create --name filemanager-builder --driver docker-container --bootstrap >/dev/null
fi
docker buildx use filemanager-builder >/dev/null

echo ""
echo "Building marketing image (${PLATFORM})..."
docker buildx build \
    --platform "${PLATFORM}" \
    --push \
    -t "${REGISTRY}:sha-${VERSION}" \
    -t "${REGISTRY}:${ENV_TAG}" \
    -f marketing/Dockerfile \
    .   # workspace root context needed for @ufop/* shared packages

echo ""
echo "============================================================"
echo " DONE — pushed to GHCR"
echo "   ${REGISTRY}:sha-${VERSION}  (immutable)"
echo "   ${REGISTRY}:${ENV_TAG}      (moving)"
echo "============================================================"
echo ""
echo "Next: ./scripts/deploy-marketing.sh"
