#!/usr/bin/env bash
# Local CI -> immutable GHCR images. No GitHub Actions and no implicit deployment.
# Usage: scripts/build-push-ghcr.sh [admin|marketing|all] [--dry-run]
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
SERVICE="${1:-all}"
[[ $# -eq 0 ]] || shift
DRY=false
if [[ "${1:-}" == --dry-run ]]; then DRY=true; shift; fi
[[ $# -eq 0 ]] || { echo "Unknown arguments" >&2; exit 2; }
case "$SERVICE" in admin|marketing) SERVICES=("$SERVICE");; all) SERVICES=(admin marketing);; *) echo "Choose admin, marketing or all" >&2; exit 2;; esac
SHA="$(git rev-parse HEAD)"
PLATFORM="${BUILD_PLATFORM:-linux/amd64}"
if [[ "$DRY" == true ]]; then
  echo "Local gate: bash scripts/verify.sh --ci"
  for service in "${SERVICES[@]}"; do echo "Build ${service}/Dockerfile for $PLATFORM -> ghcr.io/clappediya/filemanager-${service}:sha-${SHA}"; done
  exit 0
fi
# Reject dirty source before and after gates so a concurrent edit cannot be tagged as tested.
assert_source_unchanged() {
  if [[ "$(git rev-parse HEAD)" != "$SHA" ]] || [[ -n "$(git status --porcelain --untracked-files=all -- package.json pnpm-lock.yaml pnpm-workspace.yaml .npmrc .pnpmfile.cjs .dockerignore eslint.config.mjs rust-toolchain.toml CHANGELOG.md docs src src-tauri crates cli admin marketing packages scripts ':!*.tsbuildinfo' ':!admin/*.tsbuildinfo' ':!marketing/*.tsbuildinfo')" ]]; then
    echo "Commit source changes and keep the source unchanged while publishing an immutable SHA image." >&2; exit 1
  fi
}
assert_source_unchanged
bash scripts/verify.sh --ci
assert_source_unchanged
command -v gh >/dev/null
command -v docker >/dev/null
docker info >/dev/null
GHCR_USER="$(gh api user --jq .login)"
gh auth token | docker login ghcr.io -u "$GHCR_USER" --password-stdin >/dev/null
if ! docker buildx inspect filemanager-builder >/dev/null 2>&1; then
  docker buildx create --name filemanager-builder --driver docker-container --bootstrap >/dev/null
fi
for service in "${SERVICES[@]}"; do
  IMAGE="ghcr.io/clappediya/filemanager-${service}:sha-${SHA}"
  if docker buildx imagetools inspect "$IMAGE" >/dev/null 2>&1; then echo "$IMAGE already exists; refusing to overwrite an immutable tag" >&2; exit 1; fi
  docker buildx build --builder filemanager-builder --platform "$PLATFORM" --push --provenance=true --label "org.opencontainers.image.revision=${SHA}" --label "org.opencontainers.image.source=https://github.com/ClappeDiya/file-manager" --tag "$IMAGE" --file "${service}/Dockerfile" .
  docker buildx imagetools inspect "$IMAGE"
done
