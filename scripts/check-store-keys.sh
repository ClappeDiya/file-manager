#!/usr/bin/env bash
# Check Zustand persistence keys with the TypeScript parser. Supports literal constants.
# Usage: scripts/check-store-keys.sh [--list]
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
case "${1:-}" in ''|--list) ;; -h|--help) sed -n '2,3p' "$0"; exit 0;; *) echo "Unknown argument: $1" >&2; exit 2;; esac
node "$ROOT/scripts/check-store-keys.mjs" "$@"
