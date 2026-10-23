#!/usr/bin/env bash
# release-checksums.sh — write SHA256SUMS over every file of a staged
# release and sign it: SHA256SUMS.minisig (minisign format) and, with
# --gpg-key, SHA256SUMS.asc (detached, ASCII-armoured).
#
# Signing keys, first match wins:
#   --minisign-key FILE      minisign secret key, signed with the `minisign` tool
#                            (MINISIGN_PASSWORD on stdin when the key is encrypted)
#   TAURI_SIGNING_PRIVATE_KEY (+ _PASSWORD) — the updater key, signed with
#                            `pnpm tauri signer sign` (the default for releases)
#   --openssl-key DIR        an OpenSSL Ed25519 key made by
#                            scripts/lib/minisign-openssl.sh (tests, air-gapped)
#
# Usage:
#   ./scripts/release-checksums.sh --dir release-artifacts/v1.2.0 [--gpg-key KEYID]
#       [--minisign-key FILE | --openssl-key DIR] [--pubkey KEY] [--comment TEXT] [--dry-run]
#
# The result is verified with scripts/verify-release.sh (--pubkey, default:
# the updater key from src-tauri/tauri.conf.json) before the script exits.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

DIR=""
GPG_KEY="${UFOP_RELEASE_GPG_KEY:-}"
MINISIGN_KEY="${MINISIGN_SECRET_KEY_FILE:-}"
OPENSSL_KEY=""
PUBKEY=""
COMMENT=""
DRY_RUN=false

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dir) DIR="$2"; shift 2 ;;
    --gpg-key) GPG_KEY="$2"; shift 2 ;;
    --minisign-key) MINISIGN_KEY="$2"; shift 2 ;;
    --openssl-key) OPENSSL_KEY="$2"; shift 2 ;;
    --pubkey) PUBKEY="$2"; shift 2 ;;
    --comment) COMMENT="$2"; shift 2 ;;
    --dry-run) DRY_RUN=true; shift ;;
    -h|--help) sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "release-checksums: unknown argument: $1" >&2; exit 2 ;;
  esac
done
[[ -n "$DIR" && -d "$DIR" ]] || { echo "release-checksums: --dir must be an existing folder" >&2; exit 2; }

signer="none"
if [[ -n "$MINISIGN_KEY" ]]; then
  signer="minisign ($MINISIGN_KEY)"
elif [[ -n "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
  signer="tauri signer (TAURI_SIGNING_PRIVATE_KEY)"
elif [[ -n "$OPENSSL_KEY" ]]; then
  signer="openssl ($OPENSSL_KEY)"
fi

if [[ "$DRY_RUN" == true ]]; then
  echo "release-checksums plan:"
  echo "  SHA256SUMS over every file in $DIR (sorted by path)"
  echo "  SHA256SUMS.minisig with: $signer"
  [[ -n "$GPG_KEY" ]] && echo "  SHA256SUMS.asc with gpg key $GPG_KEY"
  echo "  then: scripts/verify-release.sh --dir $DIR --require-all"
  exit 0
fi
[[ "$signer" != none ]] || { echo "release-checksums: no signing key (set TAURI_SIGNING_PRIVATE_KEY, or pass --minisign-key / --openssl-key)" >&2; exit 1; }

sha256_line() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1"; else shasum -a 256 "$1"; fi
}

cd "$DIR"
rm -f SHA256SUMS SHA256SUMS.minisig SHA256SUMS.asc SHA256SUMS.sig
# Every file (sub-folders such as winget/ included), stable order.
LIST="$(mktemp)"
trap 'rm -f "$LIST"' EXIT
find . -type f ! -name 'SHA256SUMS*' ! -name '.*' -print | sed 's|^\./||' | LC_ALL=C sort > "$LIST"
: > SHA256SUMS
while IFS= read -r f; do
  sha256_line "$f" >> SHA256SUMS
done < "$LIST"
count="$(wc -l < SHA256SUMS | tr -d ' ')"
[[ "$count" -gt 0 ]] || { echo "release-checksums: no files in $DIR" >&2; exit 1; }
echo "  + SHA256SUMS ($count file(s))"

VERSION_TAG="$(basename "$DIR")"
COMMENT="${COMMENT:-timestamp:$(date +%s) file:SHA256SUMS release:${VERSION_TAG}}"
case "$signer" in
  minisign*)
    command -v minisign >/dev/null 2>&1 || { echo "release-checksums: minisign not installed" >&2; exit 1; }
    if [[ -n "${MINISIGN_PASSWORD:-}" ]]; then
      printf '%s\n' "$MINISIGN_PASSWORD" | minisign -S -s "$MINISIGN_KEY" -m SHA256SUMS -x SHA256SUMS.minisig -t "$COMMENT"
    else
      minisign -S -s "$MINISIGN_KEY" -m SHA256SUMS -x SHA256SUMS.minisig -t "$COMMENT"
    fi
    ;;
  tauri*)
    # `tauri signer sign` writes SHA256SUMS.sig: the minisign signature, base64-encoded.
    (cd "$ROOT" && pnpm -s tauri signer sign --private-key "$TAURI_SIGNING_PRIVATE_KEY" --password "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" "$DIR/SHA256SUMS" >/dev/null)
    base64 -d < SHA256SUMS.sig > SHA256SUMS.minisig
    rm -f SHA256SUMS.sig
    ;;
  openssl*)
    # shellcheck source=lib/minisign-openssl.sh
    source "$ROOT/scripts/lib/minisign-openssl.sh"
    ms_sign "$OPENSSL_KEY" SHA256SUMS "$COMMENT"
    ;;
esac
echo "  + SHA256SUMS.minisig ($signer)"

if [[ -n "$GPG_KEY" ]]; then
  command -v gpg >/dev/null 2>&1 || { echo "release-checksums: gpg not installed" >&2; exit 1; }
  gpg --batch --yes --local-user "$GPG_KEY" --armor --detach-sign --output SHA256SUMS.asc SHA256SUMS
  echo "  + SHA256SUMS.asc (gpg $GPG_KEY)"
fi

# Check what we are about to publish, exactly as users will.
verify_args=(--dir "$DIR" --require-all --quiet)
if [[ -n "$PUBKEY" ]]; then
  verify_args+=(--pubkey "$PUBKEY")
elif [[ "$signer" == openssl* ]]; then
  verify_args+=(--pubkey-file "$OPENSSL_KEY/minisign.pub")
else
  verify_args+=(--pubkey "$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["plugins"]["updater"]["pubkey"])' "$ROOT/src-tauri/tauri.conf.json")")
fi
"$ROOT/scripts/verify-release.sh" "${verify_args[@]}"
echo "  = signatures and checksums verified"
