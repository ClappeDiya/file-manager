#!/usr/bin/env bash
# release-local.sh — local-machine release pipeline for FileManager.
#
# This project does NOT use GitHub Actions (see memory: feedback-no-github-actions).
# Releases are built locally on the user's Mac, then published to GitHub Releases
# via `gh`. The Tauri updater fetches signed binaries from there.
#
# Prerequisites (verify with --check):
#   - Apple Developer signing cert installed in Keychain
#   - .env.local populated with TAURI_SIGNING_PRIVATE_KEY + APPLE_* vars
#   - `gh` authenticated (`gh auth status`)
#   - pnpm + Rust toolchain installed
#
# Usage:
#   ./scripts/release-local.sh --check         # verify prereqs only
#   ./scripts/release-local.sh --version 1.0.0 # build and publish v1.0.0
#   ./scripts/release-local.sh --version 1.0.0-rc.1 --prerelease

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION=""
PRERELEASE=false
CHECK_ONLY=false
TARGETS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --check) CHECK_ONLY=true; shift ;;
    --version) VERSION="$2"; shift 2 ;;
    --prerelease) PRERELEASE=true; shift ;;
    --target) TARGETS+=("$2"); shift 2 ;;
    *) echo "Unknown arg: $1"; exit 2 ;;
  esac
done

# Default targets if not specified — universal macOS + Linux. Windows requires
# either a Windows VM or SignPath integration (see memory: windows-signing-signpath).
if [[ ${#TARGETS[@]} -eq 0 ]]; then
  TARGETS=(aarch64-apple-darwin x86_64-apple-darwin x86_64-unknown-linux-gnu)
fi

# ---------------------------------------------------------------------------
# Prereq check
# ---------------------------------------------------------------------------
echo "==> Checking prerequisites..."

missing=0
need() {
  if ! command -v "$1" &>/dev/null; then
    echo "  MISSING: $1 not in PATH"
    missing=$((missing+1))
  fi
}
need pnpm
need cargo
need gh
need rustup

if [[ ! -f .env.local ]]; then
  echo "  MISSING: .env.local — copy from .env.example and fill in"
  missing=$((missing+1))
fi

# shellcheck disable=SC1091
[[ -f .env.local ]] && set -a && source .env.local && set +a

if [[ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
  echo "  MISSING: TAURI_SIGNING_PRIVATE_KEY in .env.local"
  missing=$((missing+1))
fi
if [[ -z "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  echo "  MISSING: APPLE_SIGNING_IDENTITY in .env.local (macOS signing will fail)"
fi
if ! gh auth status &>/dev/null; then
  echo "  MISSING: gh not authenticated — run 'gh auth login'"
  missing=$((missing+1))
fi

if [[ $missing -gt 0 ]]; then
  echo "==> $missing prerequisite(s) missing. Aborting."
  exit 1
fi
echo "==> Prereqs OK."

if [[ "$CHECK_ONLY" == true ]]; then
  exit 0
fi

if [[ -z "$VERSION" ]]; then
  echo "==> --version is required (e.g., 1.0.0 or 1.0.0-rc.1)"
  exit 2
fi

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------
echo "==> Building Tauri app for: ${TARGETS[*]}"

for target in "${TARGETS[@]}"; do
  echo "  - $target"
  rustup target add "$target" >/dev/null
  pnpm tauri build --target "$target"
done

# ---------------------------------------------------------------------------
# Publish to GitHub Releases
# ---------------------------------------------------------------------------
TAG="v${VERSION}"
echo "==> Creating GitHub release $TAG..."

artifacts=()
for target in "${TARGETS[@]}"; do
  bundle_dir="src-tauri/target/${target}/release/bundle"
  while IFS= read -r -d '' f; do
    artifacts+=("$f")
  done < <(find "$bundle_dir" -type f \( -name "*.dmg" -o -name "*.msi" -o -name "*.exe" -o -name "*.AppImage" -o -name "*.deb" -o -name "*.rpm" -o -name "*.sig" -o -name "*.tar.gz" \) -print0)
done

if [[ ${#artifacts[@]} -eq 0 ]]; then
  echo "==> No build artifacts found — did the build fail?"
  exit 1
fi

flags=(--title "FileManager $VERSION" --notes "See CHANGELOG.md")
[[ "$PRERELEASE" == true ]] && flags+=(--prerelease)

git tag "$TAG" -a -m "FileManager $VERSION" || echo "  (tag already exists)"
git push origin "$TAG"

gh release create "$TAG" "${flags[@]}" "${artifacts[@]}"

echo "==> Release published: https://github.com/ClappeDiya/file-manager/releases/tag/$TAG"
echo ""
echo "Next steps:"
echo "  1. Verify the Tauri updater can find latest.json at the GitHub Releases URL."
echo "  2. (Stable releases only) Manually PR a cask update to ClappeDiya/homebrew-tap."
echo "  3. (Windows) Once SignPath is approved, re-build the Windows targets signed."
