#!/usr/bin/env bash
# check-ipc-args.sh — tauriInvoke argument names ↔ Rust #[tauri::command]
# parameters. See scripts/check-ipc-args.mjs for the rules and why.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
exec node "$ROOT/scripts/check-ipc-args.mjs" "$@"
