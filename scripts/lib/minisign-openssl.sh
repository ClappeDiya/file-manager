#!/usr/bin/env bash
# minisign-openssl.sh — minisign-compatible keys and signatures with
# OpenSSL 3 (Ed25519 + BLAKE2b-512), for tests of the release scripts
# without the minisign tool. Source it, then:
#
#   ms_keygen DIR                 # DIR/sk.pem, DIR/minisign.pub (key id random)
#   ms_sign DIR FILE [COMMENT]    # FILE.minisig (prehashed "ED" signature)
#
# Production releases sign with the Tauri updater key (`tauri signer sign`)
# or the minisign tool; this helper exists so the verification paths can be
# tested offline.

ms_keygen() {
  local d="$1"
  mkdir -p "$d"
  openssl genpkey -algorithm ed25519 -out "$d/sk.pem" 2>/dev/null
  openssl pkey -in "$d/sk.pem" -pubout -outform DER 2>/dev/null | tail -c 32 > "$d/pk.raw"
  head -c 8 /dev/urandom > "$d/keyid"
  { printf 'Ed'; cat "$d/keyid" "$d/pk.raw"; } > "$d/pk.bin"
  printf 'untrusted comment: minisign public key %s\n%s\n' "$(od -An -tx1 "$d/keyid" | tr -d ' \n' | tr 'a-f' 'A-F')" "$(base64 -w0 < "$d/pk.bin")" > "$d/minisign.pub"
}

ms_sign() {
  local d="$1" f="$2" comment="${3:-timestamp:$(date +%s)}"
  local t; t="$(mktemp -d)"
  openssl dgst -blake2b512 -binary "$f" > "$t/h"
  openssl pkeyutl -sign -inkey "$d/sk.pem" -rawin -in "$t/h" -out "$t/sig" 2>/dev/null
  { cat "$t/sig"; printf '%s' "$comment"; } > "$t/g.msg"
  openssl pkeyutl -sign -inkey "$d/sk.pem" -rawin -in "$t/g.msg" -out "$t/g.sig" 2>/dev/null
  {
    printf 'untrusted comment: signature from minisign-openssl\n'
    { printf 'ED'; cat "$d/keyid" "$t/sig"; } | base64 -w0
    printf '\ntrusted comment: %s\n' "$comment"
    base64 -w0 < "$t/g.sig"
    printf '\n'
  } > "$f.minisig"
  rm -rf "$t"
}
