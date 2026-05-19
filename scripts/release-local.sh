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
# Asset naming: Tauri's native filenames embed productName + semver
# (e.g., "Unified File Operations Platform_0.1.0_universal.dmg"), which the
# marketing site can't link to with a stable URL. After each build this script
# copies every artifact to release-artifacts/v<version>/ under two literal
# names:
#   FileManager-<version>-<arch>.<ext>   (immutable, e.g. FileManager-0.1.0-universal.dmg)
#   FileManager-latest-<arch>.<ext>      (resolves via releases/latest/download/<name>)
# The marketing /download page links to the `-latest-` form.
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

# Default targets if not specified — universal macOS (lipo'd aarch64 + x86_64)
# plus Linux. Windows requires a Windows VM or SignPath integration (see memory:
# windows-signing-signpath). Override with one or more --target flags.
if [[ ${#TARGETS[@]} -eq 0 ]]; then
  TARGETS=(universal-apple-darwin x86_64-unknown-linux-gnu)
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
need python3

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

ensure_rust_targets() {
  local t="$1"
  if [[ "$t" == "universal-apple-darwin" ]]; then
    rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
  else
    rustup target add "$t" >/dev/null
  fi
}

for target in "${TARGETS[@]}"; do
  echo "  - $target"
  ensure_rust_targets "$target"
  pnpm tauri build --target "$target"
done

# ---------------------------------------------------------------------------
# Stage + rename to literal names
# ---------------------------------------------------------------------------
STAGE="release-artifacts/v${VERSION}"
rm -rf "$STAGE" && mkdir -p "$STAGE"
echo "==> Staging literal-name copies in $STAGE..."

# Echo "<arch>|<ext>" for a recognised Tauri bundle filename; empty otherwise.
# Order matters — more-specific patterns must precede generic ones.
literal_suffix() {
  local base; base="$(basename "$1")"
  case "$base" in
    *_universal.dmg)             echo "universal|dmg" ;;
    *_universal.app.tar.gz)      echo "universal|app.tar.gz" ;;
    *_universal.app.tar.gz.sig)  echo "universal|app.tar.gz.sig" ;;
    *.app.tar.gz)                echo "universal|app.tar.gz" ;;
    *.app.tar.gz.sig)            echo "universal|app.tar.gz.sig" ;;
    *_amd64.AppImage)            echo "amd64|AppImage" ;;
    *_amd64.AppImage.sig)        echo "amd64|AppImage.sig" ;;
    *.AppImage)                  echo "amd64|AppImage" ;;
    *.AppImage.sig)              echo "amd64|AppImage.sig" ;;
    *_amd64.deb)                 echo "amd64|deb" ;;
    *.deb)                       echo "amd64|deb" ;;
    *x86_64.rpm)                 echo "amd64|rpm" ;;
    *_x64-setup.nsis.zip.sig)    echo "x64-setup|nsis.zip.sig" ;;
    *_x64-setup.nsis.zip)        echo "x64-setup|nsis.zip" ;;
    *_x64-setup.exe)             echo "x64-setup|exe" ;;
    *_x64_*.msi)                 echo "x64|msi" ;;
    *.msi)                       echo "x64|msi" ;;
  esac
}

artifacts=()
for target in "${TARGETS[@]}"; do
  bundle_dir="src-tauri/target/${target}/release/bundle"
  if [[ ! -d "$bundle_dir" ]]; then
    echo "  WARN: no bundle dir for $target — build likely failed"
    continue
  fi
  while IFS= read -r -d '' f; do
    pair="$(literal_suffix "$f")"
    [[ -n "$pair" ]] || continue
    arch="${pair%%|*}"
    ext="${pair##*|}"
    versioned="$STAGE/FileManager-${VERSION}-${arch}.${ext}"
    latest="$STAGE/FileManager-latest-${arch}.${ext}"
    cp -p "$f" "$versioned"
    cp -p "$f" "$latest"
    artifacts+=("$versioned" "$latest")
    echo "  + $(basename "$f") → ${arch}.${ext}"
  done < <(find "$bundle_dir" -type f \
              \( -name "*.dmg" -o -name "*.app.tar.gz" -o -name "*.app.tar.gz.sig" \
              -o -name "*.AppImage" -o -name "*.AppImage.sig" \
              -o -name "*.deb" -o -name "*.rpm" \
              -o -name "*.exe" -o -name "*.nsis.zip" -o -name "*.nsis.zip.sig" \
              -o -name "*.msi" \) -print0)
done

if [[ ${#artifacts[@]} -eq 0 ]]; then
  echo "==> No build artifacts found — did the build fail?"
  exit 1
fi

# ---------------------------------------------------------------------------
# Generate Tauri updater manifest (latest.json)
# ---------------------------------------------------------------------------
PUB_DATE="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
python3 - "$STAGE" "$VERSION" "$PUB_DATE" <<'PY' > "$STAGE/latest.json"
import json, os, sys
stage, version, pub_date = sys.argv[1], sys.argv[2], sys.argv[3]
def sig(name):
    p = os.path.join(stage, name)
    return open(p).read() if os.path.exists(p) else None

base = "https://github.com/ClappeDiya/file-manager/releases/latest/download"
platforms = {}

mac = sig("FileManager-latest-universal.app.tar.gz.sig")
if mac:
    entry = {"signature": mac, "url": f"{base}/FileManager-latest-universal.app.tar.gz"}
    platforms["darwin-aarch64"] = entry
    platforms["darwin-x86_64"]  = entry

linux = sig("FileManager-latest-amd64.AppImage.sig")
if linux:
    platforms["linux-x86_64"] = {
        "signature": linux,
        "url": f"{base}/FileManager-latest-amd64.AppImage",
    }

win = sig("FileManager-latest-x64-setup.nsis.zip.sig")
if win:
    platforms["windows-x86_64"] = {
        "signature": win,
        "url": f"{base}/FileManager-latest-x64-setup.nsis.zip",
    }

print(json.dumps({
    "version": version,
    "notes": f"See https://github.com/ClappeDiya/file-manager/releases/tag/v{version}",
    "pub_date": pub_date,
    "platforms": platforms,
}, indent=2))
PY
artifacts+=("$STAGE/latest.json")
plat_summary="$(python3 -c "import json,sys; print(','.join(sorted(json.load(open(sys.argv[1]))['platforms'].keys())) or 'none')" "$STAGE/latest.json")"
echo "  + latest.json (platforms: $plat_summary)"

# ---------------------------------------------------------------------------
# Publish to GitHub Releases
# ---------------------------------------------------------------------------
TAG="v${VERSION}"
echo "==> Creating GitHub release $TAG with ${#artifacts[@]} asset(s)..."

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
