#!/usr/bin/env bash
# check-tauri-plugins.sh — verify every `tauri-plugin-*` dependency in
# src-tauri/Cargo.toml has a matching `.plugin(tauri_plugin_X::...)` call
# in src-tauri/src/lib.rs.
#
# Why this exists:
#   A Tauri plugin only takes effect after `tauri::Builder::default()` is
#   chained with `.plugin(tauri_plugin_X::init())` (or the Builder-pattern
#   variant). Adding `tauri-plugin-clipboard-manager = "2"` to Cargo.toml
#   pulls the crate into the binary but its commands/events stay dormant
#   until the plugin is actually registered. The compiler does not warn —
#   the project compiles, runs, and ONLY the plugin's APIs fail at runtime
#   with confusing "command not found" / "no such permission" errors. Same
#   DNA as iter 5/9/10/11/12/13: contract drift that compiles cleanly.
#
# What it checks (one direction — see "What it does NOT check" below):
#   * For every `tauri-plugin-X` entry in `[dependencies]` of
#     `src-tauri/Cargo.toml`, there is at least one `.plugin(tauri_plugin_X::`
#     call in `src-tauri/src/lib.rs` (kebab → snake normalization).
#   * Both call-shapes accepted:
#       .plugin(tauri_plugin_X::init())
#       .plugin(tauri_plugin_X::Builder::new().build())
#       .plugin(tauri_plugin_X::Builder::default().build())
#       — any call starting with `.plugin(tauri_plugin_X::` counts.
#
# What it does NOT check:
#   * The reverse direction (every .plugin() call has a Cargo entry) —
#     this is already compiler-enforced. `cargo check` fails to resolve
#     `tauri_plugin_X` if X is not in Cargo.toml.
#   * Plugins from `[dev-dependencies]` or `[build-dependencies]` — those
#     are not runtime plugins and need no `.plugin()` call.
#   * `[target.X.dependencies]` blocks (platform-specific deps) — those
#     are usually paired with `#[cfg(...)] .plugin(...)` blocks, and the
#     compile error from cfg drift surfaces immediately on the matching
#     platform. Out of scope for a portable check.
#   * Commented-out entries (`# tauri-plugin-foo = "..."`) — skipped.
#
# Pure-bash + awk, no jq / python. Sub-second. Bash 3.2 compatible.
#
# Usage:
#   ./scripts/check-tauri-plugins.sh         # verify; non-zero on missing init
#   ./scripts/check-tauri-plugins.sh --list  # debug: print parsed sets

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CARGO="src-tauri/Cargo.toml"
LIB="src-tauri/src/lib.rs"

LIST=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)  LIST=true; shift ;;
    -h|--help)
      sed -n '2,40p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "Unknown arg: $1 (try --help)"; exit 2 ;;
  esac
done

for f in "$CARGO" "$LIB"; do
  [[ -f "$f" ]] || { echo "ERROR: $f not found" >&2; exit 1; }
done

# ---------------------------------------------------------------------------
# 1. Parse Cargo.toml plugin entries.
#    Scope: only inside the top-level `[dependencies]` section. We do NOT
#    look at `[dev-dependencies]`, `[build-dependencies]`, or
#    `[target.X.dependencies]`. Stops at the next section header `[...]`.
#
#    Pre-processing: strip line comments (`# ...`) AFTER preserving any
#    `#` that appears inside a string literal — but for Cargo.toml plugin
#    entries this is a degenerate case (plugin names don't contain `#`),
#    so a plain `sed 's/#.*//'` is safe and we apply it. A commented-out
#    `# tauri-plugin-foo = "2"` is therefore correctly skipped.
# ---------------------------------------------------------------------------
declared="$(awk '
BEGIN { in_deps = 0 }
/^\[dependencies\][[:space:]]*$/ { in_deps = 1; next }
/^\[[^]]+\]/ { in_deps = 0; next }  # any other section header
in_deps {
  # Strip line comment
  pos = index($0, "#")
  if (pos > 0) $0 = substr($0, 1, pos - 1)
  # Match: tauri-plugin-NAME = ...
  if (match($0, /^[[:space:]]*tauri-plugin-[A-Za-z0-9_-]+[[:space:]]*=/)) {
    line = substr($0, RSTART, RLENGTH)
    # Extract the name (strip leading whitespace + trailing whitespace/=)
    sub(/^[[:space:]]+/, "", line)
    sub(/[[:space:]]*=$/, "", line)
    print line
  }
}
' "$CARGO" | sort -u)"

if [[ -z "$declared" ]]; then
  echo "OK: 0 tauri-plugin-* dependencies in $CARGO (nothing to check)"
  exit 0
fi

# ---------------------------------------------------------------------------
# 2. Parse lib.rs for .plugin(tauri_plugin_X::...) calls.
#    Snake case in Rust source; normalize to kebab case for comparison.
#
#    Comment stripping (in this order):
#      a) `/* ... */` block comments — may span multiple lines.
#      b) `// ...` line comments — rest of line.
#    Without (a) a contributor who block-comments out a `.plugin()` call
#    during refactoring would silently pass the check. We do NOT strip
#    string contents — `".plugin(tauri_plugin_X::init())"` inside a Rust
#    string literal is contrived and would still register correctly if
#    the string is later eval'd, which Tauri does not do.
# ---------------------------------------------------------------------------
registered="$(awk '
BEGIN { in_block = 0 }
{
  line = $0; out = ""; n = length(line); i = 1
  while (i <= n) {
    c = substr(line, i, 1)
    if (in_block) {
      if (c == "*" && substr(line, i + 1, 1) == "/") { in_block = 0; i += 2; continue }
      i++; continue
    }
    if (c == "/" && substr(line, i + 1, 1) == "*") { in_block = 1; i += 2; continue }
    if (c == "/" && substr(line, i + 1, 1) == "/") { break }
    out = out c
    i++
  }
  print out
}' "$LIB" \
  | grep -oE '\.plugin\(tauri_plugin_[A-Za-z0-9_]+' \
  | sed -E 's|^\.plugin\(tauri_plugin_||' \
  | tr '_' '-' \
  | awk '{ print "tauri-plugin-" $0 }' \
  | sort -u)"

if [[ "$LIST" == true ]]; then
  echo "Cargo.toml plugins ($(echo "$declared" | grep -c .)):"
  echo "$declared" | sed 's/^/  /'
  echo "lib.rs .plugin() calls ($(echo "$registered" | grep -c .)):"
  echo "$registered" | sed 's/^/  /'
fi

# ---------------------------------------------------------------------------
# 3. Compare — every declared plugin must appear in registered.
# ---------------------------------------------------------------------------
missing="$(comm -23 <(echo "$declared") <(echo "$registered"))"

failed=0
if [[ -n "$missing" ]]; then
  while IFS= read -r p; do
    [[ -z "$p" ]] && continue
    snake="$(echo "$p" | sed 's/^tauri-plugin-//' | tr '-' '_')"
    echo "FAIL: $p is a Cargo.toml dependency but has no \`.plugin(tauri_plugin_${snake}::init())\` call in $LIB" >&2
    echo "      The plugin compiles into the binary but stays dormant — its commands and events never fire." >&2
    failed=$((failed + 1))
  done <<< "$missing"
fi

if [[ "$failed" -eq 0 ]]; then
  count="$(echo "$declared" | grep -c .)"
  echo "OK: $count Tauri plugin(s), all registered via .plugin() in $LIB"
  exit 0
fi
exit 1
