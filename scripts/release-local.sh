#!/usr/bin/env bash
# release-local.sh — local-machine release pipeline for FileManager.
#
# This project does NOT use GitHub Actions (see .github/workflows/README.md).
# Releases are built on the release machine(s), then published to GitHub
# Releases via `gh`. The Tauri updater fetches signed binaries from there.
#
# What a release contains (see docs/release-checklist.md):
#   - desktop bundles for every target this machine can build (macOS
#     universal .dmg/.app.tar.gz on a Mac; .deb/.rpm/.AppImage on Linux;
#     .msi + NSIS setup.exe on Windows, or NSIS only when cross-building with
#     cargo-xwin), copied to stable literal names
#   - the CLI packages: ufop-<version>-<target>.tar.gz/.zip (ufop, ufop-server,
#     shell completions, man pages), ufop_<version>_<arch>.deb on Linux, and a
#     Homebrew formula (ufop.rb) on macOS
#   - winget manifests when a Windows installer was built
#   - latest.json (Tauri updater), SHA256SUMS + SHA256SUMS.minisig (and
#     SHA256SUMS.asc with --gpg-key) over every uploaded file
#
# Prerequisites (verify with --check):
#   - .env.local (or the environment) with TAURI_SIGNING_PRIVATE_KEY (+ _PASSWORD)
#   - macOS: Apple Developer signing cert in the Keychain, APPLE_* vars
#   - Windows signing (optional): WINDOWS_SIGN_CERT_FILE + WINDOWS_SIGN_CERT_PASSWORD
#     (a .pfx) or WINDOWS_SIGN_CERT_THUMBPRINT (certificate store; signtool only)
#   - `gh` authenticated (`gh auth status`) unless --no-publish
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
#   ./scripts/release-local.sh --check                    # verify prereqs only
#   ./scripts/release-local.sh --print-plan               # target matrix: what would be built or skipped, and why
#   ./scripts/release-local.sh --version 1.0.0            # build every target this host can build, publish v1.0.0
#   ./scripts/release-local.sh --version 1.0.0 --dry-run  # print every command, run nothing
#   ./scripts/release-local.sh --version 1.0.0-rc.1 --prerelease
#
# Options:
#   --target T          build only T (repeatable): universal-apple-darwin,
#                       x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu,
#                       x86_64-pc-windows-msvc
#   --windows / --no-windows
#                       require / leave out the Windows target (default: built
#                       when this host can build it)
#   --windows-cross xwin
#                       build Windows from Linux/macOS with cargo-xwin (NSIS
#                       only; needs cargo-xwin, makensis, clang-cl and lld-link).
#                       Also UFOP_WINDOWS_CROSS=xwin.
#   --no-cli            skip the CLI packages
#   --gpg-key KEYID     also sign SHA256SUMS with GPG (SHA256SUMS.asc)
#   --no-publish        build and stage only (no tag, no GitHub release)

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION=""
PRERELEASE=false
CHECK_ONLY=false
PRINT_PLAN=false
DRY_RUN=false
PUBLISH=true
WITH_CLI=true
WINDOWS=auto
WINDOWS_CROSS="${UFOP_WINDOWS_CROSS:-}"
GPG_KEY="${UFOP_RELEASE_GPG_KEY:-}"
TARGETS=()

usage() { sed -n '2,62p' "$0" | sed 's/^# \{0,1\}//'; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --check) CHECK_ONLY=true; shift ;;
    --version) VERSION="$2"; shift 2 ;;
    --prerelease) PRERELEASE=true; shift ;;
    --target) TARGETS+=("$2"); shift 2 ;;
    --windows) WINDOWS=yes; shift ;;
    --no-windows) WINDOWS=no; shift ;;
    --windows-cross) WINDOWS_CROSS="$2"; shift 2 ;;
    --no-cli) WITH_CLI=false; shift ;;
    --gpg-key) GPG_KEY="$2"; shift 2 ;;
    --no-publish) PUBLISH=false; shift ;;
    --print-plan) PRINT_PLAN=true; shift ;;
    --dry-run) DRY_RUN=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown arg: $1"; exit 2 ;;
  esac
done

case "$WINDOWS_CROSS" in ""|xwin) ;; *) echo "--windows-cross: only 'xwin' is supported"; exit 2 ;; esac

# Overridable for tests of the plan on other hosts.
HOST_OS="${UFOP_RELEASE_HOST_OS:-}"
if [[ -z "$HOST_OS" ]]; then
  case "$(uname -s)" in
    Darwin) HOST_OS=macos ;;
    Linux) HOST_OS=linux ;;
    MINGW*|MSYS*|CYGWIN*) HOST_OS=windows ;;
    *) HOST_OS=unknown ;;
  esac
fi
HOST_ARCH="${UFOP_RELEASE_HOST_ARCH:-$(uname -m)}"
case "$HOST_ARCH" in arm64) HOST_ARCH=aarch64 ;; amd64) HOST_ARCH=x86_64 ;; esac

have() { command -v "$1" >/dev/null 2>&1; }

ALL_TARGETS=(universal-apple-darwin x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-pc-windows-msvc)

# "yes|how" or "no|why" for a target on this host.
target_status() {
  case "$1" in
    universal-apple-darwin|aarch64-apple-darwin|x86_64-apple-darwin)
      if [[ "$HOST_OS" == macos ]]; then echo "yes|native (Xcode, codesign, notarization)"
      else echo "no|needs a Mac (Xcode toolchain, codesign and notarization)"; fi ;;
    x86_64-unknown-linux-gnu|aarch64-unknown-linux-gnu)
      local arch="${1%%-*}"
      if [[ "$HOST_OS" == linux && "$HOST_ARCH" == "$arch" ]]; then echo "yes|native (.deb, .rpm, .AppImage)"
      elif [[ "$HOST_OS" == linux ]]; then echo "no|needs an ${arch} Linux machine (WebKitGTK cannot be cross-linked without a sysroot)"
      else echo "no|needs Linux (WebKitGTK); build on an ${arch} Linux machine or VM"; fi ;;
    x86_64-pc-windows-msvc)
      if [[ "$HOST_OS" == windows ]]; then echo "yes|native (MSI + NSIS)"
      elif [[ "$WINDOWS_CROSS" == xwin ]]; then
        local miss=()
        have cargo-xwin || miss+=(cargo-xwin)
        have makensis || miss+=(makensis)
        have clang-cl || miss+=(clang-cl)
        have lld-link || miss+=(lld-link)
        if [[ ${#miss[@]} -eq 0 ]]; then echo "yes|cross with cargo-xwin (NSIS only; MSI needs WiX on Windows)"
        else echo "no|--windows-cross xwin needs ${miss[*]}"; fi
      else echo "no|needs a Windows host, or --windows-cross xwin (cargo-xwin, makensis, clang-cl, lld-link)"; fi ;;
    *) echo "no|unknown target" ;;
  esac
}

# Choose targets: explicit --target, else everything buildable here.
SKIPPED=()
if [[ ${#TARGETS[@]} -eq 0 ]]; then
  for t in "${ALL_TARGETS[@]}"; do
    st="$(target_status "$t")"
    if [[ "$t" == *windows* && "$WINDOWS" == no ]]; then
      SKIPPED+=("$t|left out (--no-windows)")
    elif [[ "${st%%|*}" == yes ]]; then
      TARGETS+=("$t")
    elif [[ "$t" == *windows* && "$WINDOWS" == yes ]]; then
      TARGETS+=("$t")
    else
      SKIPPED+=("$t|${st#*|}")
    fi
  done
fi

# Windows code signing (Tauri's bundle.windows.signCommand signs every binary
# and installer during bundling, before the updater signature is made).
windows_sign_command() {
  # The wrappers read the certificate and password from the environment, so
  # no secret ends up in the Tauri config or the process list of tauri.
  if [[ "$HOST_OS" == windows ]]; then
    if [[ -n "${WINDOWS_SIGN_CERT_THUMBPRINT:-}${WINDOWS_SIGN_CERT_FILE:-}" ]]; then
      echo "powershell -NoProfile -ExecutionPolicy Bypass -File ${ROOT}/packaging/windows/sign.ps1 %1"
    fi
  elif [[ -n "${WINDOWS_SIGN_CERT_FILE:-}" ]] && have osslsigncode; then
    echo "${ROOT}/packaging/windows/sign.sh %1"
  fi
}
windows_signing_summary() {
  local c; c="$(windows_sign_command)"
  if [[ -n "$c" ]]; then echo "signed with $([[ "$c" == powershell* ]] && echo "signtool (packaging/windows/sign.ps1)" || echo "osslsigncode (packaging/windows/sign.sh)"), $([[ -n "${WINDOWS_SIGN_CERT_THUMBPRINT:-}" ]] && echo "certificate store" || echo "${WINDOWS_SIGN_CERT_FILE:-}"), timestamp ${WINDOWS_SIGN_TIMESTAMP_URL:-http://timestamp.digicert.com}"
  elif [[ -n "${WINDOWS_SIGN_CERT_FILE:-}${WINDOWS_SIGN_CERT_THUMBPRINT:-}" ]]; then echo "UNSIGNED: a certificate is set but neither signtool (Windows) nor osslsigncode (cross) is usable"
  else echo "unsigned (set WINDOWS_SIGN_CERT_FILE + WINDOWS_SIGN_CERT_PASSWORD, or WINDOWS_SIGN_CERT_THUMBPRINT)"; fi
}

print_plan() {
  local v="${VERSION:-$(python3 -c 'import json; print(json.load(open("package.json"))["version"])' 2>/dev/null || echo "<version>")}"
  echo "==> Release plan for v${v} on ${HOST_OS}/${HOST_ARCH}"
  echo ""
  printf '  %-28s %-6s %s\n' "TARGET" "BUILD" "HOW / WHY NOT"
  for t in "${TARGETS[@]}"; do
    local st; st="$(target_status "$t")"
    if [[ "${st%%|*}" == yes ]]; then
      printf '  %-28s %-6s %s\n' "$t" "yes" "${st#*|}"
    else
      printf '  %-28s %-6s %s\n' "$t" "FORCED" "requested, but will fail here: ${st#*|}"
    fi
  done
  for s in ${SKIPPED[@]+"${SKIPPED[@]}"}; do
    printf '  %-28s %-6s %s\n' "${s%%|*}" "skip" "${s#*|}"
  done
  echo ""
  echo "  Desktop artifacts: FileManager-${v}-<arch>.<ext> and FileManager-latest-<arch>.<ext>, latest.json"
  if [[ "$WITH_CLI" == true ]]; then
    for t in "${TARGETS[@]}"; do
      case "$t" in
        *windows*) echo "  CLI ${t}: ufop-${v}-${t}.zip (ufop.exe, ufop-server.exe, completions, man pages)" ;;
        *linux*) echo "  CLI ${t}: ufop-${v}-${t}.tar.gz + ufop_${v}_$([[ "$t" == x86_64* ]] && echo amd64 || echo arm64).deb (bash/zsh/fish completions, man pages)" ;;
        *apple*) echo "  CLI ${t}: ufop-${v}-universal-apple-darwin.tar.gz + Homebrew formula ufop.rb" ;;
      esac
    done
  else
    echo "  CLI packages: skipped (--no-cli)"
  fi
  if printf '%s\n' "${TARGETS[@]}" | grep -q windows; then
    echo "  Windows signing: $(windows_signing_summary)"
    echo "  winget: ClappeDiya.FileManager.yaml, .installer.yaml, .locale.en-US.yaml (packaging/windows/winget/generate-manifest.sh)"
  fi
  echo "  Checksums: SHA256SUMS over every uploaded file + SHA256SUMS.minisig ($([[ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]] && echo "updater key" || echo "updater key: TAURI_SIGNING_PRIVATE_KEY not set yet"))$([[ -n "$GPG_KEY" ]] && echo " + SHA256SUMS.asc (gpg ${GPG_KEY})")"
  echo "  Verified before upload with scripts/verify-release.sh --require-all"
  if [[ "$PUBLISH" == true ]]; then
    echo "  Publish: git tag v${v} + gh release create v${v}$([[ "$PRERELEASE" == true ]] && echo " --prerelease") with every file in release-artifacts/v${v}/"
  else
    echo "  Publish: no (--no-publish); files stay in release-artifacts/v${v}/"
  fi
}

if [[ "$PRINT_PLAN" == true ]]; then
  print_plan
  exit 0
fi

# ---------------------------------------------------------------------------
# Prereq check
# ---------------------------------------------------------------------------
echo "==> Checking prerequisites..."

missing=0
need() {
  if ! have "$1"; then
    echo "  MISSING: $1 not in PATH"
    missing=$((missing+1))
  fi
}
need pnpm
need cargo
need rustup
need python3
[[ "$PUBLISH" == true ]] && need gh

if [[ -f .env.local ]]; then
  # shellcheck disable=SC1091
  set -a && source .env.local && set +a
elif [[ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
  echo "  MISSING: .env.local — copy from .env.example and fill in"
  missing=$((missing+1))
fi

if [[ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
  echo "  MISSING: TAURI_SIGNING_PRIVATE_KEY in .env.local (updater signatures and SHA256SUMS.minisig)"
  missing=$((missing+1))
fi
if printf '%s\n' "${TARGETS[@]}" | grep -q apple && [[ -z "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  echo "  MISSING: APPLE_SIGNING_IDENTITY in .env.local (macOS signing will fail)"
fi
if printf '%s\n' "${TARGETS[@]}" | grep -q windows; then
  echo "  Windows: $(windows_signing_summary)"
fi
if [[ -n "$GPG_KEY" ]] && ! have gpg; then
  echo "  MISSING: gpg (for --gpg-key)"
  missing=$((missing+1))
fi
if [[ "$PUBLISH" == true ]] && have gh && ! gh auth status &>/dev/null; then
  echo "  MISSING: gh not authenticated — run 'gh auth login'"
  missing=$((missing+1))
fi
for t in "${TARGETS[@]}"; do
  st="$(target_status "$t")"
  if [[ "${st%%|*}" != yes ]]; then
    echo "  CANNOT BUILD: $t — ${st#*|}"
    missing=$((missing+1))
  fi
done
for s in ${SKIPPED[@]+"${SKIPPED[@]}"}; do
  echo "  skip: ${s%%|*} — ${s#*|}"
done

if [[ $missing -gt 0 && "$DRY_RUN" != true ]]; then
  echo "==> $missing prerequisite(s) missing. Aborting."
  exit 1
fi
echo "==> Prereqs $([[ $missing -gt 0 ]] && echo "incomplete ($missing; ignored for --dry-run)" || echo OK)."

if [[ "$CHECK_ONLY" == true ]]; then
  exit 0
fi

if [[ -z "$VERSION" ]]; then
  echo "==> --version is required (e.g., 1.0.0 or 1.0.0-rc.1)"
  exit 2
fi
if [[ ${#TARGETS[@]} -eq 0 ]]; then
  echo "==> Nothing this host can build (see --print-plan)."
  exit 1
fi

run() {
  if [[ "$DRY_RUN" == true ]]; then
    printf '+'
    printf ' %q' "$@"
    printf '\n'
  else
    "$@"
  fi
}

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------
echo "==> Building for: ${TARGETS[*]}"

ensure_rust_targets() {
  local t="$1"
  if [[ "$t" == "universal-apple-darwin" ]]; then
    run rustup target add aarch64-apple-darwin x86_64-apple-darwin
  else
    run rustup target add "$t"
  fi
}

sign_config() {
  local c; c="$(windows_sign_command)"
  [[ -n "$c" ]] || return 0
  python3 -c 'import json,sys; print(json.dumps({"bundle": {"windows": {"signCommand": sys.argv[1]}}}))' "$c"
}

# Sign a CLI executable the same way Tauri signs the desktop binaries.
sign_exe() {
  local c; c="$(windows_sign_command)"
  [[ -n "$c" ]] || return 0
  # shellcheck disable=SC2086
  run ${c//%1/$1}
}

CLI_OUT="target/release-cli"
HOST_UFOP=""
HOST_SERVER=""
build_cli() {
  local t="$1" out="$CLI_OUT/$t"
  run mkdir -p "$out"
  case "$t" in
    universal-apple-darwin)
      for a in aarch64-apple-darwin x86_64-apple-darwin; do
        run cargo build --release --manifest-path cli/Cargo.toml --target "$a"
        run cargo build --release --manifest-path src-tauri/Cargo.toml --bin ufop-server --target "$a"
      done
      run lipo -create -output "$out/ufop" cli/target/aarch64-apple-darwin/release/ufop cli/target/x86_64-apple-darwin/release/ufop
      run lipo -create -output "$out/ufop-server" src-tauri/target/aarch64-apple-darwin/release/ufop-server src-tauri/target/x86_64-apple-darwin/release/ufop-server
      ;;
    *windows*)
      local cargo_cmd=(cargo build)
      [[ "$HOST_OS" != windows ]] && cargo_cmd=(cargo xwin build)
      run "${cargo_cmd[@]}" --release --manifest-path cli/Cargo.toml --target "$t"
      run "${cargo_cmd[@]}" --release --manifest-path src-tauri/Cargo.toml --bin ufop-server --target "$t"
      run cp "cli/target/$t/release/ufop.exe" "src-tauri/target/$t/release/ufop-server.exe" "$out/"
      sign_exe "$out/ufop.exe"
      sign_exe "$out/ufop-server.exe"
      ;;
    *)
      run cargo build --release --manifest-path cli/Cargo.toml --target "$t"
      run cargo build --release --manifest-path src-tauri/Cargo.toml --bin ufop-server --target "$t"
      run cp "cli/target/$t/release/ufop" "src-tauri/target/$t/release/ufop-server" "$out/"
      ;;
  esac
}

for target in "${TARGETS[@]}"; do
  echo "  - $target"
  ensure_rust_targets "$target"
  case "$target" in
    *windows*)
      tauri_args=(tauri build --target "$target")
      if [[ "$HOST_OS" != windows ]]; then
        tauri_args+=(--runner cargo-xwin --bundles nsis)
      fi
      cfg="$(sign_config)"
      [[ -n "$cfg" ]] && tauri_args+=(--config "$cfg")
      run pnpm "${tauri_args[@]}"
      ;;
    *) run pnpm tauri build --target "$target" ;;
  esac
  if [[ "$WITH_CLI" == true ]]; then
    build_cli "$target"
  fi
done

# A ufop for this machine generates completions and man pages for every package.
if [[ "$WITH_CLI" == true ]]; then
  run cargo build --release --manifest-path cli/Cargo.toml --bin ufop
  run cargo build --release --manifest-path src-tauri/Cargo.toml --bin ufop-server
  HOST_EXE=""
  [[ "$HOST_OS" == windows ]] && HOST_EXE=".exe"
  HOST_UFOP="cli/target/release/ufop${HOST_EXE}"
  HOST_SERVER="src-tauri/target/release/ufop-server${HOST_EXE}"
fi

if [[ "$DRY_RUN" == true ]]; then
  STAGE="release-artifacts/v${VERSION}"
  echo "==> Then (dry run, nothing staged):"
  echo "+ stage desktop bundles as FileManager-${VERSION}-<arch>.<ext> / FileManager-latest-<arch>.<ext> in $STAGE"
  if [[ "$WITH_CLI" == true ]]; then
    for t in "${TARGETS[@]}"; do
      deb=()
      [[ "$t" == *linux* ]] && deb=(--deb)
      run packaging/linux/package-cli.sh --version "$VERSION" --target "$t" --bin-dir "$CLI_OUT/$t" --gen-bin "$HOST_UFOP" --gen-server-bin "$HOST_SERVER" --out "$STAGE" ${deb[@]+"${deb[@]}"}
    done
    printf '%s\n' "${TARGETS[@]}" | grep -q apple && echo "+ packaging/macos/generate-formula.sh --version $VERSION --url …/ufop-${VERSION}-universal-apple-darwin.tar.gz --sha256 <sha> --out $STAGE/ufop.rb"
  fi
  printf '%s\n' "${TARGETS[@]}" | grep -q windows && echo "+ packaging/windows/winget/generate-manifest.sh --version $VERSION --installer-url …/FileManager-${VERSION}-x64-setup.exe --installer-file $STAGE/FileManager-${VERSION}-x64-setup.exe --out $STAGE"
  echo "+ write $STAGE/latest.json"
  run scripts/release-checksums.sh --dir "$STAGE" ${GPG_KEY:+--gpg-key "$GPG_KEY"}
  if [[ "$PUBLISH" == true ]]; then
    run git tag "v${VERSION}" -a -m "FileManager ${VERSION}"
    run git push origin "v${VERSION}"
    echo "+ gh release create v${VERSION} --title 'FileManager ${VERSION}' --notes 'See CHANGELOG.md'$([[ "$PRERELEASE" == true ]] && echo " --prerelease") $STAGE/*"
  fi
  exit 0
fi

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
    *_aarch64.AppImage)          echo "arm64|AppImage" ;;
    *_aarch64.AppImage.sig)      echo "arm64|AppImage.sig" ;;
    *_amd64.AppImage)            echo "amd64|AppImage" ;;
    *_amd64.AppImage.sig)        echo "amd64|AppImage.sig" ;;
    *.AppImage)                  echo "amd64|AppImage" ;;
    *.AppImage.sig)              echo "amd64|AppImage.sig" ;;
    *_arm64.deb)                 echo "arm64|deb" ;;
    *_amd64.deb)                 echo "amd64|deb" ;;
    *.deb)                       echo "amd64|deb" ;;
    *aarch64.rpm)                echo "arm64|rpm" ;;
    *x86_64.rpm)                 echo "amd64|rpm" ;;
    *_x64-setup.nsis.zip.sig)    echo "x64-setup|nsis.zip.sig" ;;
    *_x64-setup.nsis.zip)        echo "x64-setup|nsis.zip" ;;
    *_x64-setup.exe.sig)         echo "x64-setup|exe.sig" ;;
    *_x64-setup.exe)             echo "x64-setup|exe" ;;
    *_x64_*.msi.sig)             echo "x64|msi.sig" ;;
    *_x64_*.msi)                 echo "x64|msi" ;;
    *.msi)                       echo "x64|msi" ;;
  esac
}

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
    cp -p "$f" "$STAGE/FileManager-${VERSION}-${arch}.${ext}"
    cp -p "$f" "$STAGE/FileManager-latest-${arch}.${ext}"
    echo "  + $(basename "$f") → ${arch}.${ext}"
  done < <(find "$bundle_dir" -type f \
              \( -name "*.dmg" -o -name "*.app.tar.gz" -o -name "*.app.tar.gz.sig" \
              -o -name "*.AppImage" -o -name "*.AppImage.sig" \
              -o -name "*.deb" -o -name "*.rpm" \
              -o -name "*.exe" -o -name "*.exe.sig" -o -name "*.nsis.zip" -o -name "*.nsis.zip.sig" \
              -o -name "*.msi" -o -name "*.msi.sig" \) -print0)
done

if [[ -z "$(ls -A "$STAGE")" ]]; then
  echo "==> No build artifacts found — did the build fail?"
  exit 1
fi

# ---------------------------------------------------------------------------
# CLI packages, Homebrew formula, winget manifests
# ---------------------------------------------------------------------------
RELEASE_URL="https://github.com/ClappeDiya/file-manager/releases/download/v${VERSION}"
sha256_of() {
  if have sha256sum; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

if [[ "$WITH_CLI" == true ]]; then
  echo "==> CLI packages"
  for t in "${TARGETS[@]}"; do
    deb=()
    [[ "$t" == *linux* ]] && deb=(--deb)
    packaging/linux/package-cli.sh --version "$VERSION" --target "$t" --bin-dir "$CLI_OUT/$t" \
      --gen-bin "$HOST_UFOP" --gen-server-bin "$HOST_SERVER" --out "$STAGE" ${deb[@]+"${deb[@]}"}
  done
  mac_tgz="$STAGE/ufop-${VERSION}-universal-apple-darwin.tar.gz"
  if [[ -f "$mac_tgz" ]]; then
    packaging/macos/generate-formula.sh --version "$VERSION" --url "${RELEASE_URL}/$(basename "$mac_tgz")" --sha256 "$(sha256_of "$mac_tgz")" --out "$STAGE/ufop.rb"
  fi
fi

win_exe="$STAGE/FileManager-${VERSION}-x64-setup.exe"
if [[ -f "$win_exe" ]]; then
  echo "==> winget manifests"
  msi_args=()
  win_msi="$STAGE/FileManager-${VERSION}-x64.msi"
  if [[ -f "$win_msi" ]]; then
    msi_args=(--msi-url "${RELEASE_URL}/$(basename "$win_msi")" --msi-sha256 "$(sha256_of "$win_msi")")
  fi
  packaging/windows/winget/generate-manifest.sh --version "$VERSION" --installer-url "${RELEASE_URL}/$(basename "$win_exe")" \
    --installer-file "$win_exe" --out "$STAGE" ${msi_args[@]+"${msi_args[@]}"}
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

linux_arm = sig("FileManager-latest-arm64.AppImage.sig")
if linux_arm:
    platforms["linux-aarch64"] = {
        "signature": linux_arm,
        "url": f"{base}/FileManager-latest-arm64.AppImage",
    }

# Tauri 2 updates Windows from the installer itself (.exe.sig); older
# bundles shipped a .nsis.zip.
win = sig("FileManager-latest-x64-setup.exe.sig")
win_url = f"{base}/FileManager-latest-x64-setup.exe"
if not win:
    win = sig("FileManager-latest-x64-setup.nsis.zip.sig")
    win_url = f"{base}/FileManager-latest-x64-setup.nsis.zip"
if win:
    platforms["windows-x86_64"] = {"signature": win, "url": win_url}

print(json.dumps({
    "version": version,
    "notes": f"See https://github.com/ClappeDiya/file-manager/releases/tag/v{version}",
    "pub_date": pub_date,
    "platforms": platforms,
}, indent=2))
PY
plat_summary="$(python3 -c "import json,sys; print(','.join(sorted(json.load(open(sys.argv[1]))['platforms'].keys())) or 'none')" "$STAGE/latest.json")"
echo "  + latest.json (platforms: $plat_summary)"

# ---------------------------------------------------------------------------
# SHA256SUMS + signatures (verified before anything is uploaded)
# ---------------------------------------------------------------------------
echo "==> Checksums and signatures"
scripts/release-checksums.sh --dir "$STAGE" ${GPG_KEY:+--gpg-key "$GPG_KEY"}

artifacts=()
while IFS= read -r -d '' f; do
  artifacts+=("$f")
done < <(find "$STAGE" -maxdepth 1 -type f -print0 | sort -z)

if [[ "$PUBLISH" != true ]]; then
  echo "==> Staged ${#artifacts[@]} file(s) in $STAGE (--no-publish)."
  exit 0
fi

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
echo "  2. Check the download: scripts/verify-release.sh --dir <folder with the downloads>"
echo "  3. (Stable releases only) Commit ufop.rb to ClappeDiya/homebrew-tap (Formula/ufop.rb)."
echo "  4. (Windows) Submit the winget manifests: wingetcreate submit <folder with the three ClappeDiya.FileManager*.yaml>"
echo "  5. Multi-arch server image: scripts/docker-build.sh --push --tag v${VERSION}"
