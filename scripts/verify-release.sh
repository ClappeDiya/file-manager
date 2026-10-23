#!/usr/bin/env bash
# verify-release.sh — check a downloaded FileManager / UFOP release.
#
# Verifies the minisign signature of SHA256SUMS (and, when present and gpg
# is installed, the detached GPG signature SHA256SUMS.asc), then checks the
# SHA-256 of every downloaded file listed in SHA256SUMS.
#
# Usage:
#   ./verify-release.sh [--dir DIR] [--pubkey KEY | --pubkey-file FILE]
#                       [--sums SHA256SUMS] [--sig SHA256SUMS.minisig]
#                       [--require-all] [--no-gpg] [--quiet]
#
#   --dir DIR          folder with the downloads and SHA256SUMS (default: .)
#   --pubkey KEY       minisign public key (the `RW…` line, a whole
#                      minisign.pub, or Tauri's base64 form). Default: the
#                      release key below (also in src-tauri/tauri.conf.json,
#                      plugins.updater.pubkey).
#   --require-all      fail when a listed file was not downloaded (default:
#                      check only the files that are present)
#   --no-gpg           skip SHA256SUMS.asc even when it exists
#
# The signature is checked with the first tool found: `minisign`, then
# `ufop verify-release` (the UFOP CLI), then `openssl` (3.0 or newer: Ed25519
# and BLAKE2b-512). Exit code 0 = everything verified, 1 = a check failed,
# 2 = usage error or no tool to verify with.

set -euo pipefail

# Release signing key (minisign public key of the updater, key id F5189997328AA80E).
DEFAULT_PUBKEY="RWQOqigyl5kY9Vj4LeRcZ2DMlAEY85jWjDqOZ94gBgYhSd1TOsJ0nxzZ"

DIR="."
PUBKEY=""
SUMS=""
SIG=""
REQUIRE_ALL=false
USE_GPG=true
QUIET=false

usage() { sed -n '2,27p' "$0" | sed 's/^# \{0,1\}//'; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dir) DIR="$2"; shift 2 ;;
    --pubkey) PUBKEY="$2"; shift 2 ;;
    --pubkey-file) PUBKEY="$(cat "$2")"; shift 2 ;;
    --sums) SUMS="$2"; shift 2 ;;
    --sig) SIG="$2"; shift 2 ;;
    --require-all) REQUIRE_ALL=true; shift ;;
    --no-gpg) USE_GPG=false; shift ;;
    --quiet) QUIET=true; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "verify-release: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

say() { [[ "$QUIET" == true ]] || echo "$@"; }

SUMS="${SUMS:-$DIR/SHA256SUMS}"
SIG="${SIG:-$SUMS.minisig}"
[[ -f "$SUMS" ]] || { echo "verify-release: $SUMS not found" >&2; exit 2; }
[[ -f "$SIG" ]] || { echo "verify-release: $SIG not found" >&2; exit 2; }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# The `RW…` base64 line of a public key given in any of the accepted forms.
pubkey_line() {
  local k="${1:-$DEFAULT_PUBKEY}"
  if [[ "$k" == *"untrusted comment"* ]]; then
    printf '%s\n' "$k" | sed -n '2p' | tr -d '\r'
  elif [[ "$k" == RW* ]]; then
    printf '%s\n' "$k" | tr -d '\r\n '
  else
    # Tauri stores the whole minisign.pub base64-encoded.
    printf '%s' "$k" | base64 -d 2>/dev/null | sed -n '2p' | tr -d '\r'
  fi
}

# A minisign signature file; Tauri's `.sig` form (base64 of it) is decoded.
signature_file() {
  if head -n1 "$1" | grep -q '^untrusted comment:'; then
    cat "$1"
  else
    base64 -d < "$1"
  fi
}

PK_LINE="$(pubkey_line "$PUBKEY")"
[[ -n "$PK_LINE" ]] || { echo "verify-release: cannot read the public key" >&2; exit 2; }
printf 'untrusted comment: minisign public key\n%s\n' "$PK_LINE" > "$TMP/key.pub"
signature_file "$SIG" > "$TMP/sums.minisig" || { echo "verify-release: cannot read $SIG" >&2; exit 2; }

# Ed25519 verification of file $2 against raw signature $3 with the raw
# 32-byte public key $1 (openssl 3).
ed25519_verify() {
  printf '\x30\x2a\x30\x05\x06\x03\x2b\x65\x70\x03\x21\x00' > "$TMP/pub.der"
  cat "$1" >> "$TMP/pub.der"
  openssl pkey -pubin -inform DER -in "$TMP/pub.der" -out "$TMP/pub.pem" 2>/dev/null || return 1
  openssl pkeyutl -verify -pubin -inkey "$TMP/pub.pem" -rawin -in "$2" -sigfile "$3" >/dev/null 2>&1
}

verify_with_openssl() {
  local sigline tcline gsline
  printf '%s' "$PK_LINE" | base64 -d > "$TMP/pk.bin" 2>/dev/null || return 1
  sigline="$(sed -n '2p' "$TMP/sums.minisig" | tr -d '\r')"
  tcline="$(sed -n '3p' "$TMP/sums.minisig" | tr -d '\r')"
  gsline="$(sed -n '4p' "$TMP/sums.minisig" | tr -d '\r')"
  printf '%s' "$sigline" | base64 -d > "$TMP/sig.bin" 2>/dev/null || return 1
  [[ "$(wc -c < "$TMP/pk.bin")" -eq 42 && "$(wc -c < "$TMP/sig.bin")" -eq 74 ]] || { echo "  malformed key or signature" >&2; return 1; }
  # Key ids (bytes 3–10) must match.
  if ! cmp -s <(head -c 10 "$TMP/pk.bin" | tail -c 8) <(head -c 10 "$TMP/sig.bin" | tail -c 8); then
    echo "  the signature was made with a different key" >&2
    return 1
  fi
  tail -c 32 "$TMP/pk.bin" > "$TMP/pk.raw"
  tail -c 64 "$TMP/sig.bin" > "$TMP/sig.raw"
  local alg; alg="$(head -c 2 "$TMP/sig.bin")"
  case "$alg" in
    ED) openssl dgst -blake2b512 -binary "$SUMS" > "$TMP/msg" ;;
    Ed) cp "$SUMS" "$TMP/msg" ;;
    *) echo "  unknown signature algorithm" >&2; return 1 ;;
  esac
  ed25519_verify "$TMP/pk.raw" "$TMP/msg" "$TMP/sig.raw" || return 1
  # The trusted comment is signed too (signature || comment).
  [[ "$tcline" == "trusted comment: "* ]] || return 1
  { cat "$TMP/sig.raw"; printf '%s' "${tcline#trusted comment: }"; } > "$TMP/global.msg"
  printf '%s' "$gsline" | base64 -d > "$TMP/global.sig" 2>/dev/null || return 1
  ed25519_verify "$TMP/pk.raw" "$TMP/global.msg" "$TMP/global.sig" || return 1
  say "  ${tcline}"
}

say "==> Signature of $(basename "$SUMS")"
if command -v minisign >/dev/null 2>&1; then
  minisign -Vm "$SUMS" -p "$TMP/key.pub" -x "$TMP/sums.minisig" >/dev/null || { echo "FAIL: bad minisign signature" >&2; exit 1; }
  say "  OK (minisign)"
elif command -v ufop >/dev/null 2>&1 && ufop verify-release --help >/dev/null 2>&1; then
  ufop verify-release "$SUMS" "$TMP/sums.minisig" --pubkey "$PK_LINE" --signature-only >/dev/null || { echo "FAIL: bad minisign signature" >&2; exit 1; }
  say "  OK (ufop verify-release)"
elif command -v openssl >/dev/null 2>&1 && openssl list -digest-algorithms 2>/dev/null | grep -qi blake2b512; then
  verify_with_openssl || { echo "FAIL: bad minisign signature" >&2; exit 1; }
  say "  OK (openssl)"
else
  echo "verify-release: install minisign (https://jedisct1.github.io/minisign/), the ufop CLI, or OpenSSL 3 to verify" >&2
  exit 2
fi

ASC="$SUMS.asc"
if [[ "$USE_GPG" == true && -f "$ASC" ]]; then
  say "==> GPG signature $(basename "$ASC")"
  if command -v gpg >/dev/null 2>&1; then
    gpg --batch --verify "$ASC" "$SUMS" 2>"$TMP/gpg.log" || { cat "$TMP/gpg.log" >&2; echo "FAIL: bad GPG signature (import the release key first)" >&2; exit 1; }
    say "  OK (gpg)"
  else
    say "  skipped (gpg not installed)"
  fi
fi

say "==> Checksums"
sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1
  fi
}
checked=0
failed=0
missing=0
base="$(dirname "$SUMS")"
while IFS= read -r line || [[ -n "$line" ]]; do
  line="${line%$'\r'}"
  [[ -z "$line" || "$line" == \#* ]] && continue
  hash="${line%% *}"
  name="${line#* }"
  name="${name# }"
  name="${name#\*}"
  if [[ ! -f "$base/$name" ]]; then
    missing=$((missing + 1))
    if [[ "$REQUIRE_ALL" == true ]]; then
      echo "  MISSING $name" >&2
      failed=$((failed + 1))
    fi
    continue
  fi
  if [[ "$(sha256_of "$base/$name")" == "$hash" ]]; then
    say "  OK      $name"
    checked=$((checked + 1))
  else
    echo "  FAILED  $name" >&2
    failed=$((failed + 1))
  fi
done < "$SUMS"

if (( failed > 0 )); then
  echo "FAIL: $failed file(s) did not verify" >&2
  exit 1
fi
if (( checked == 0 )); then
  echo "FAIL: none of the files listed in $(basename "$SUMS") is in $base" >&2
  exit 1
fi
say "==> Verified $checked file(s)$( (( missing > 0 )) && echo ", $missing listed file(s) not downloaded" )."
