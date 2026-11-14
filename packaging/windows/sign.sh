#!/usr/bin/env bash
# sign.sh — Authenticode-sign one Windows file from Linux/macOS with
# osslsigncode (cross builds; scripts/release-local.sh --windows-cross xwin).
# Used as Tauri's bundle.windows.signCommand: packaging/windows/sign.sh <file>
#   WINDOWS_SIGN_CERT_FILE      .pfx with the code-signing certificate and key
#   WINDOWS_SIGN_CERT_PASSWORD  its password
#   WINDOWS_SIGN_TIMESTAMP_URL  RFC 3161 timestamp server (default http://timestamp.digicert.com)
set -euo pipefail
f="${1:?usage: sign.sh <file>}"
: "${WINDOWS_SIGN_CERT_FILE:?WINDOWS_SIGN_CERT_FILE is not set}"
command -v osslsigncode >/dev/null 2>&1 || { echo "sign.sh: osslsigncode is not installed" >&2; exit 1; }
tmp="${f}.signing"
osslsigncode sign -pkcs12 "$WINDOWS_SIGN_CERT_FILE" -pass "${WINDOWS_SIGN_CERT_PASSWORD:-}" \
  -h sha256 -n "FileManager" -i "https://filemanager.clappe.com" \
  -ts "${WINDOWS_SIGN_TIMESTAMP_URL:-http://timestamp.digicert.com}" -in "$f" -out "$tmp"
mv -f "$tmp" "$f"
osslsigncode verify -in "$f" >/dev/null
