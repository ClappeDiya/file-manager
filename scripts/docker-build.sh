#!/usr/bin/env bash
# docker-build.sh — build the ufop-server image for several platforms with
# docker buildx and publish ONE manifest list to GHCR.
#
# Same local publishing model as scripts/build-push-marketing.sh (no GitHub
# Actions): `gh auth token` logs in to ghcr.io, a dedicated buildx builder
# (docker-container driver) does the build, and `--push` uploads every
# platform under the same tags.
#
# Usage:
#   ./scripts/docker-build.sh [--platforms linux/amd64,linux/arm64] [--push]
#                             [--tag TAG]... [--image ghcr.io/OWNER/ufop-server]
#                             [--builder NAME] [--append-node ssh://arm-host]
#                             [--no-cache] [--dry-run]
#
#   --platforms   comma-separated (default: linux/amd64,linux/arm64)
#   --push        push the manifest list (default: build only; a single
#                 platform without --push is loaded into the local daemon)
#   --tag         extra tag (repeatable); always tagged sha-<commit>, and
#                 v<version> from package.json when HEAD is tagged v<version>
#   --append-node add a native node (e.g. an arm64 machine over SSH) to the
#                 builder so that platform is not emulated
#   --dry-run     print the plan and every command without running anything
#
# Foreign platforms are emulated with QEMU (binfmt_misc); the script
# installs the handlers with tonistiigi/binfmt when the builder lacks them.
# See docs/release-checklist.md ("Docker images") for build times.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PLATFORMS="linux/amd64,linux/arm64"
PUSH=false
DRY_RUN=false
NO_CACHE=false
IMAGE="${UFOP_SERVER_IMAGE:-ghcr.io/clappediya/ufop-server}"
BUILDER="${UFOP_BUILDX_BUILDER:-ufop-multiarch}"
APPEND_NODES=()
EXTRA_TAGS=()
DOCKERFILE="docker/ufop-server.Dockerfile"

usage() { sed -n '2,29p' "$0" | sed 's/^# \{0,1\}//'; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --platforms) PLATFORMS="$2"; shift 2 ;;
    --push) PUSH=true; shift ;;
    --tag) EXTRA_TAGS+=("$2"); shift 2 ;;
    --image) IMAGE="$2"; shift 2 ;;
    --builder) BUILDER="$2"; shift 2 ;;
    --append-node) APPEND_NODES+=("$2"); shift 2 ;;
    --no-cache) NO_CACHE=true; shift ;;
    --dry-run|--print-plan) DRY_RUN=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "docker-build: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

# Validate platforms (the Dockerfile maps TARGETARCH amd64/arm64 only).
IFS=',' read -r -a PLATFORM_LIST <<< "$PLATFORMS"
for p in "${PLATFORM_LIST[@]}"; do
  case "$p" in
    linux/amd64|linux/arm64) ;;
    *) echo "docker-build: unsupported platform '$p' (linux/amd64, linux/arm64)" >&2; exit 2 ;;
  esac
done

run() {
  if [[ "$DRY_RUN" == true ]]; then
    printf '+'
    printf ' %q' "$@"
    printf '\n'
  else
    "$@"
  fi
}

SHA="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
VERSION="$(python3 -c 'import json; print(json.load(open("package.json"))["version"])' 2>/dev/null || echo 0.0.0)"
TAGS=("sha-${SHA}")
if git describe --exact-match --tags HEAD 2>/dev/null | grep -qx "v${VERSION}"; then
  TAGS+=("v${VERSION}")
fi
TAGS+=(${EXTRA_TAGS[@]+"${EXTRA_TAGS[@]}"})

HOST_ARCH="$(uname -m)"
case "$HOST_ARCH" in x86_64|amd64) HOST_PLATFORM=linux/amd64 ;; aarch64|arm64) HOST_PLATFORM=linux/arm64 ;; *) HOST_PLATFORM="linux/$HOST_ARCH" ;; esac

echo "============================================================"
echo " ufop-server image"
echo "   image:      ${IMAGE}"
echo "   tags:       ${TAGS[*]}"
echo "   platforms:  ${PLATFORMS}"
for p in "${PLATFORM_LIST[@]}"; do
  if [[ "$p" == "$HOST_PLATFORM" ]]; then
    echo "     - $p native"
  elif [[ ${#APPEND_NODES[@]} -gt 0 ]]; then
    echo "     - $p native node if one of ${APPEND_NODES[*]} provides it, else QEMU"
  else
    echo "     - $p QEMU emulation (slow; see --append-node)"
  fi
done
echo "   builder:    ${BUILDER} (docker-container driver)"
echo "   output:     $([[ "$PUSH" == true ]] && echo "push one manifest list" || { [[ ${#PLATFORM_LIST[@]} -eq 1 ]] && echo "load into the local daemon" || echo "build cache only (add --push to publish)"; })"
echo "   dockerfile: ${DOCKERFILE}"
echo "============================================================"

if [[ "$DRY_RUN" != true ]]; then
  command -v docker >/dev/null 2>&1 || { echo "docker-build: docker is not installed" >&2; exit 1; }
  docker buildx version >/dev/null 2>&1 || { echo "docker-build: docker buildx is not available" >&2; exit 1; }
fi

if [[ "$PUSH" == true ]]; then
  if [[ "$DRY_RUN" == true ]]; then
    echo "+ gh auth token | docker login ghcr.io -u <gh user> --password-stdin"
  else
    command -v gh >/dev/null 2>&1 || { echo "install gh CLI: https://cli.github.com/" >&2; exit 1; }
    token="$(gh auth token 2>/dev/null || true)"
    [[ -n "$token" ]] || { echo "Run: gh auth login --scopes write:packages" >&2; exit 1; }
    user="$(gh api user --jq .login)"
    printf '%s\n' "$token" | docker login "${IMAGE%%/*}" -u "$user" --password-stdin >/dev/null
  fi
fi

# Builder: create once (docker-container driver builds manifest lists).
if [[ "$DRY_RUN" == true ]] || ! docker buildx inspect "$BUILDER" >/dev/null 2>&1; then
  run docker buildx create --name "$BUILDER" --driver docker-container --bootstrap
fi
for node in ${APPEND_NODES[@]+"${APPEND_NODES[@]}"}; do
  run docker buildx create --name "$BUILDER" --append "$node"
done
run docker buildx use "$BUILDER"

# QEMU handlers for platforms the builder cannot run yet.
if [[ "$DRY_RUN" == true ]]; then
  for p in "${PLATFORM_LIST[@]}"; do
    [[ "$p" == "$HOST_PLATFORM" ]] || echo "+ docker run --privileged --rm tonistiigi/binfmt --install ${p#linux/}   # only when the builder lacks $p"
  done
else
  supported="$(docker buildx inspect --bootstrap "$BUILDER" | sed -n 's/^Platforms: *//p' | tr -d ' ')"
  for p in "${PLATFORM_LIST[@]}"; do
    if [[ ",$supported," != *",$p,"* && ",$supported," != *",$p/"* ]]; then
      echo "==> Installing QEMU emulation for $p"
      docker run --privileged --rm tonistiigi/binfmt --install "${p#linux/}"
    fi
  done
fi

args=(buildx build --platform "$PLATFORMS" -f "$DOCKERFILE"
  --label "org.opencontainers.image.source=https://github.com/ClappeDiya/file-manager"
  --label "org.opencontainers.image.revision=$(git rev-parse HEAD 2>/dev/null || echo unknown)"
  --label "org.opencontainers.image.version=${VERSION}")
for t in "${TAGS[@]}"; do
  args+=(-t "${IMAGE}:${t}")
done
[[ "$NO_CACHE" == true ]] && args+=(--no-cache)
# Behind a TLS-intercepting proxy, pass the CA the same way as docker/with-ca.sh expects.
if [[ -n "${UFOP_BUILD_CA:-}" ]]; then
  args+=(--secret "id=ca,src=${UFOP_BUILD_CA}")
fi
if [[ "$PUSH" == true ]]; then
  args+=(--push)
elif [[ ${#PLATFORM_LIST[@]} -eq 1 ]]; then
  args+=(--load)
fi
args+=(.)
run docker "${args[@]}"

if [[ "$PUSH" == true ]]; then
  run docker buildx imagetools inspect "${IMAGE}:${TAGS[0]}"
  echo "==> Pushed ${IMAGE}:{${TAGS[*]}} (one manifest list: ${PLATFORMS})"
fi
