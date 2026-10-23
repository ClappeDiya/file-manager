#!/usr/bin/env bash
# test-release-scripts.sh — offline tests of the local release tooling.
#
# Exercises, without building installers or touching the network:
#   - release-local.sh --print-plan on Linux, macOS and Windows hosts (simulated)
#     and --dry-run (every command printed, nothing run)
#   - docker-build.sh --dry-run (one multi-arch manifest list) and its platform check
#   - packaging/linux/package-cli.sh (a real tarball + .deb when a debug `ufop`
#     exists in cli/target/debug, else its --dry-run plan)
#   - packaging/windows/winget/generate-manifest.sh and packaging/macos/generate-formula.sh
#   - release-checksums.sh with a throwaway OpenSSL Ed25519 key (minisign format)
#     and a throwaway GPG key, then verify-release.sh (OpenSSL path and, when
#     built, `ufop verify-release`), including tamper detection
#
# Usage: ./scripts/test-release-scripts.sh     (exit 0 = all passed)

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

PASS=0
FAIL=0
T="$(mktemp -d)"
GH=""
cleanup() {
  [[ -n "$GH" ]] && GNUPGHOME="$GH" gpgconf --kill gpg-agent >/dev/null 2>&1
  rm -rf "$T" ${GH:+"$GH"}
}
trap cleanup EXIT

ok() { PASS=$((PASS + 1)); echo "  ok   $1"; }
bad() { FAIL=$((FAIL + 1)); echo "  FAIL $1"; }
# expect_contains NAME OUTPUT NEEDLE...
expect_contains() {
  local name="$1" out="$2"
  shift 2
  for n in "$@"; do
    if [[ "$out" != *"$n"* ]]; then
      bad "$name: missing '$n'"
      return
    fi
  done
  ok "$name"
}

echo "==> release-local.sh --print-plan"
out="$(UFOP_RELEASE_HOST_OS=linux UFOP_RELEASE_HOST_ARCH=x86_64 env -u UFOP_WINDOWS_CROSS scripts/release-local.sh --print-plan 2>&1)"
expect_contains "linux host plan" "$out" "x86_64-unknown-linux-gnu     yes" "universal-apple-darwin       skip" "x86_64-pc-windows-msvc       skip" "needs a Windows host" "ufop_0" ".deb" "SHA256SUMS.minisig"
out="$(UFOP_RELEASE_HOST_OS=windows scripts/release-local.sh --print-plan --gpg-key 0xABC 2>&1)"
expect_contains "windows host plan" "$out" "x86_64-pc-windows-msvc       yes    native (MSI + NSIS)" "winget" "Windows signing: unsigned" "SHA256SUMS.asc (gpg 0xABC)" ".zip"
out="$(UFOP_RELEASE_HOST_OS=windows WINDOWS_SIGN_CERT_FILE=c.pfx scripts/release-local.sh --print-plan 2>&1)"
expect_contains "windows signing plan" "$out" "signtool (packaging/windows/sign.ps1)"
out="$(UFOP_RELEASE_HOST_OS=macos UFOP_RELEASE_HOST_ARCH=arm64 scripts/release-local.sh --print-plan --no-windows --no-publish 2>&1)"
expect_contains "macos host plan" "$out" "universal-apple-darwin       yes" "left out (--no-windows)" "Homebrew formula ufop.rb" "Publish: no"
out="$(UFOP_RELEASE_HOST_OS=linux UFOP_RELEASE_HOST_ARCH=x86_64 scripts/release-local.sh --print-plan --windows-cross xwin 2>&1)"
expect_contains "cross plan names missing tools" "$out" "--windows-cross xwin needs"
out="$(UFOP_RELEASE_HOST_OS=linux UFOP_RELEASE_HOST_ARCH=x86_64 scripts/release-local.sh --print-plan --target x86_64-pc-windows-msvc 2>&1)"
expect_contains "forced target is flagged" "$out" "FORCED" "will fail here"

echo "==> release-local.sh --dry-run"
out="$(UFOP_RELEASE_HOST_OS=linux UFOP_RELEASE_HOST_ARCH=x86_64 TAURI_SIGNING_PRIVATE_KEY=x scripts/release-local.sh --version 9.8.7 --dry-run --no-publish 2>&1)"
code=$?
if [[ $code -eq 0 ]]; then ok "dry run exits 0"; else bad "dry run exit $code"; fi
expect_contains "dry run commands" "$out" "+ pnpm tauri build --target x86_64-unknown-linux-gnu" "+ packaging/linux/package-cli.sh --version 9.8.7" "+ scripts/release-checksums.sh --dir release-artifacts/v9.8.7"
if [[ ! -d release-artifacts/v9.8.7 ]]; then ok "dry run stages nothing"; else bad "dry run created release-artifacts/v9.8.7"; rm -rf release-artifacts/v9.8.7; fi
out="$(UFOP_RELEASE_HOST_OS=windows TAURI_SIGNING_PRIVATE_KEY=x WINDOWS_SIGN_CERT_THUMBPRINT=AB scripts/release-local.sh --version 9.8.7 --dry-run --no-publish 2>&1)"
expect_contains "windows dry run signs via signCommand" "$out" "signCommand" "sign.ps1 target/release-cli/x86_64-pc-windows-msvc/ufop.exe" "generate-manifest.sh --version 9.8.7"

echo "==> docker-build.sh --dry-run"
out="$(scripts/docker-build.sh --dry-run --push --tag edge 2>&1)"
expect_contains "docker plan" "$out" "--platform linux/amd64\\,linux/arm64" "--push" ":edge" "docker buildx create --name ufop-multiarch" "tonistiigi/binfmt" "one manifest list"
if scripts/docker-build.sh --dry-run --platforms linux/riscv64 >/dev/null 2>&1; then bad "docker rejects unknown platforms"; else ok "docker rejects unknown platforms"; fi
out="$(scripts/docker-build.sh --dry-run --platforms linux/amd64 2>&1)"
expect_contains "single platform loads locally" "$out" "--load"
if grep -q 'uname -m' docker/ufop-server.Dockerfile; then bad "Dockerfile still uses uname -m"; else ok "Dockerfile uses TARGETARCH"; fi
expect_contains "Dockerfile multi-arch stages" "$(cat docker/ufop-server.Dockerfile)" 'FROM --platform=$BUILDPLATFORM' "ARG TARGETARCH" 'id=ufop-server-target-${TARGETARCH}'

echo "==> packaging"
if [[ -x cli/target/debug/ufop ]]; then
  mkdir -p "$T/bin"
  cp cli/target/debug/ufop "$T/bin/"
  if packaging/linux/package-cli.sh --version 9.8.7 --target x86_64-unknown-linux-gnu --bin-dir "$T/bin" --out "$T/pkg" --deb >/dev/null; then
    list="$(tar -tzf "$T/pkg/ufop-9.8.7-x86_64-unknown-linux-gnu.tar.gz")"
    expect_contains "cli tarball" "$list" "bin/ufop" "share/bash-completion/completions/ufop" "share/zsh/site-functions/_ufop" "share/fish/vendor_completions.d/ufop.fish" "share/man/man1/ufop-backup-bench.1" "install.sh"
    if command -v dpkg-deb >/dev/null 2>&1; then
      expect_contains "cli deb" "$(dpkg-deb -c "$T/pkg/ufop_9.8.7_amd64.deb")" "./usr/share/bash-completion/completions/ufop" "./usr/share/zsh/vendor-completions/_ufop" "./usr/share/man/man1/ufop.1.gz"
    fi
  else
    bad "package-cli.sh"
  fi
else
  out="$(packaging/linux/package-cli.sh --version 9.8.7 --target x86_64-unknown-linux-gnu --bin-dir "$T/none" --deb --dry-run)"
  expect_contains "package-cli plan (no debug ufop built)" "$out" "completions:" "ufop_9.8.7_amd64.deb"
fi
printf 'installer' > "$T/setup.exe"
if packaging/windows/winget/generate-manifest.sh --version 9.8.7 --installer-url https://example.invalid/FileManager-9.8.7-x64-setup.exe --installer-file "$T/setup.exe" --out "$T/winget" >/dev/null; then
  inst="$(cat "$T/winget/ClappeDiya.FileManager.installer.yaml")"
  sha="$(sha256sum "$T/setup.exe" | cut -d' ' -f1 | tr 'a-f' 'A-F')"
  expect_contains "winget installer manifest" "$inst" "PackageVersion: 9.8.7" "InstallerType: nsis" "InstallerSha256: $sha" "ManifestType: installer"
  expect_contains "winget version manifest" "$(cat "$T/winget/ClappeDiya.FileManager.yaml")" "DefaultLocale: en-US" "ManifestType: version"
  expect_contains "winget locale manifest" "$(cat "$T/winget/ClappeDiya.FileManager.locale.en-US.yaml")" "PackageName: FileManager" "ManifestType: defaultLocale"
else
  bad "generate-manifest.sh"
fi
if packaging/windows/winget/generate-manifest.sh --version 1.0 --installer-url u --sha256 abc >/dev/null 2>&1; then bad "winget validates input"; else ok "winget validates input"; fi
out="$(packaging/macos/generate-formula.sh --version 9.8.7 --url https://example.invalid/u.tar.gz --sha256 "$(printf '%064d' 7)")"
expect_contains "homebrew formula" "$out" "class Ufop < Formula" "bash_completion.install" "zsh_completion.install" "fish_completion.install" "man1.install" 'sha256 "0000000000000000000000000000000000000000000000000000000000000007"'

echo "==> SHA256SUMS signing and verification"
if openssl list -digest-algorithms 2>/dev/null | grep -qi blake2b512; then
  # shellcheck source=lib/minisign-openssl.sh
  source scripts/lib/minisign-openssl.sh
  ms_keygen "$T/key"
  mkdir -p "$T/rel"
  printf 'one' > "$T/rel/FileManager-9.8.7-amd64.deb"
  printf 'two' > "$T/rel/ufop-9.8.7-x86_64-unknown-linux-gnu.tar.gz"
  gpg_args=()
  if command -v gpg >/dev/null 2>&1; then
    GH="$(mktemp -d /tmp/ufopgpg.XXXX)"
    chmod 700 "$GH"
    if GNUPGHOME="$GH" gpg --batch --passphrase '' --quick-gen-key "UFOP Release Test <release-test@example.invalid>" ed25519 sign never >/dev/null 2>&1; then
      gpg_args=(--gpg-key release-test@example.invalid)
    fi
  fi
  if GNUPGHOME="${GH:-${GNUPGHOME:-}}" env -u TAURI_SIGNING_PRIVATE_KEY -u MINISIGN_SECRET_KEY_FILE scripts/release-checksums.sh --dir "$T/rel" --openssl-key "$T/key" ${gpg_args[@]+"${gpg_args[@]}"} >/dev/null; then
    ok "release-checksums.sh signs and self-verifies"
  else
    bad "release-checksums.sh"
  fi
  [[ "$(wc -l < "$T/rel/SHA256SUMS")" -eq 2 ]] && ok "SHA256SUMS lists every file" || bad "SHA256SUMS line count"
  [[ ${#gpg_args[@]} -eq 0 || -f "$T/rel/SHA256SUMS.asc" ]] && ok "GPG signature (when gpg is installed)" || bad "SHA256SUMS.asc missing"
  if GNUPGHOME="${GH:-${GNUPGHOME:-}}" PATH=/usr/bin:/bin scripts/verify-release.sh --dir "$T/rel" --pubkey-file "$T/key/minisign.pub" --quiet; then ok "verify-release.sh (openssl)"; else bad "verify-release.sh (openssl)"; fi
  if [[ -x cli/target/debug/ufop ]]; then
    if cli/target/debug/ufop verify-release "$T/rel/SHA256SUMS" --pubkey-file "$T/key/minisign.pub" >/dev/null; then ok "ufop verify-release"; else bad "ufop verify-release"; fi
  fi
  ms_keygen "$T/other"
  if PATH=/usr/bin:/bin scripts/verify-release.sh --dir "$T/rel" --pubkey-file "$T/other/minisign.pub" --quiet --no-gpg 2>/dev/null; then bad "a different key is rejected"; else ok "a different key is rejected"; fi
  printf 'tampered' > "$T/rel/FileManager-9.8.7-amd64.deb"
  if PATH=/usr/bin:/bin scripts/verify-release.sh --dir "$T/rel" --pubkey-file "$T/key/minisign.pub" --quiet --no-gpg 2>/dev/null; then bad "a tampered file is rejected"; else ok "a tampered file is rejected"; fi
  printf 'one' > "$T/rel/FileManager-9.8.7-amd64.deb"
  echo "0000000000000000000000000000000000000000000000000000000000000000  extra" >> "$T/rel/SHA256SUMS"
  if PATH=/usr/bin:/bin scripts/verify-release.sh --dir "$T/rel" --pubkey-file "$T/key/minisign.pub" --quiet --no-gpg 2>/dev/null; then bad "an edited SHA256SUMS is rejected"; else ok "an edited SHA256SUMS is rejected"; fi
else
  echo "  skip OpenSSL 3 with BLAKE2b-512 is not available"
fi
out="$(scripts/release-checksums.sh --dir "$T" --dry-run --gpg-key K)"
expect_contains "release-checksums plan" "$out" "SHA256SUMS over every file" "SHA256SUMS.asc with gpg key K"

echo "==> $PASS passed, $FAIL failed"
[[ $FAIL -eq 0 ]]
