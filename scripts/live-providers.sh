#!/usr/bin/env bash
# live-providers.sh — run the live-provider contract tests (backend
# compatibility + backup → list → restore → verify → delete → compact →
# cleanup) against every provider whose UFOP_LIVE_* variables are set.
# Unset providers are skipped and listed with the variables they need.
#
# Usage:
#   ./scripts/live-providers.sh                 # real providers from the environment
#   UFOP_LIVE_USE_MOCKS=1 ./scripts/live-providers.sh   # plus the in-process mocks
#
# Variables: docs/backup-destinations.md, "Live provider tests".
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../src-tauri"
exec cargo test --lib backup_engine::stores::live_providers -- --nocapture --test-threads=1
