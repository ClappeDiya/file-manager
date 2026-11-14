#!/usr/bin/env bash
# Rewrite the snapshot of `ufop`'s commands that the desktop terminal's help
# drawer ships with (crates/ufop-core/data/ufop-cli-help.json). Run it after
# adding or changing a CLI command or option; cli/tests/help_snapshot.rs
# fails until the snapshot matches.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
OUT="crates/ufop-core/data/ufop-cli-help.json"
TMP="$(mktemp)"
trap 'rm -f "$TMP"' EXIT
(cd cli && cargo run --quiet -- help-json) > "$TMP"
# The CLI's own output is what the test compares with; keep a trailing newline for tidy diffs.
cp "$TMP" "$OUT"
echo "Wrote $OUT ($(grep -o '"path"' "$OUT" | wc -l) commands)"
