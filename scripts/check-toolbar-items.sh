#!/usr/bin/env bash
# check-toolbar-items.sh — verify the ALL_TOOLBAR_ITEMS array in
# src/components/file-manager.tsx stays in lockstep with every
# `toolbarItems.includes("…")` callsite in the same file.
#
# Why this exists:
#   CLAUDE.md's end-to-end checklist for adding a new panel says:
#     "Toolbar items array (`ALL_TOOLBAR_ITEMS`) controls the toolbar
#     customizer UI — every panel toggle needs an entry."
#   Two failure modes the compiler can't catch:
#     (a) Add a `toolbarItems.includes("foo")` gate around a panel toggle,
#         but forget to register "foo" in ALL_TOOLBAR_ITEMS — the panel
#         exists but the user has no way to enable it from the customizer.
#     (b) Leave a stale entry in ALL_TOOLBAR_ITEMS after the panel/toggle
#         was removed — the customizer shows an option that does nothing.
#   Both ship cleanly past lint, typecheck, and tests. Same DNA as iter
#   5/9/10/11/12 — silent contract mismatches.
#
# What it checks:
#   * Set A = `id: "X"` strings inside the ALL_TOOLBAR_ITEMS array literal.
#   * Set B = `X` strings inside any `toolbarItems.includes("X")` callsite
#     in the file (literal-string form only; dynamic refs like
#     `toolbarItems.includes(item.id)` are ignored — those are the
#     customizer-loop iterator and are well-formed by construction).
#   * Fails if A and B differ in either direction.
#
# Pure-bash + awk, no jq / python. Sub-second. Bash 3.2 compatible.
#
# Usage:
#   ./scripts/check-toolbar-items.sh         # verify; non-zero on mismatch
#   ./scripts/check-toolbar-items.sh --list  # debug: print parsed sets

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

FILE="src/components/file-manager.tsx"

LIST=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)  LIST=true; shift ;;
    -h|--help)
      sed -n '2,35p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "Unknown arg: $1 (try --help)"; exit 2 ;;
  esac
done

if [[ ! -f "$FILE" ]]; then
  echo "ERROR: $FILE not found" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Strip TS/JSX block and line comments before pattern matching, so a
# commented-out includes() call or id entry cannot trigger a spurious
# finding. We do NOT strip string contents — the patterns of interest
# (the toolbar IDs) live inside string literals.
#
# Two stripping passes:
#   1. Block comments /* ... */ (may span lines)
#   2. Line comments // ...   (one line)
#
# Done as a single awk pass with two states: in_block (cross-line) and
# in_string (single-line, used to avoid stripping `//` inside a string).
# ---------------------------------------------------------------------------
AWK_FILE="$(mktemp -t check-toolbar-items.XXXXXX.awk)"
STRIP_FILE="$(mktemp -t check-toolbar-items-stripped.XXXXXX)"
trap 'rm -f "$AWK_FILE" "$STRIP_FILE"' EXIT

cat > "$AWK_FILE" <<'AWK_EOF'
BEGIN {
  in_block = 0
  in_str = ""   # "", '', or `` (backtick)
}
{
  line = $0
  out = ""
  n = length(line)
  i = 1
  BT = sprintf("%c", 96)  # backtick
  while (i <= n) {
    c = substr(line, i, 1)
    if (in_block) {
      if (c == "*" && substr(line, i + 1, 1) == "/") { in_block = 0; i += 2; continue }
      i++; continue
    }
    if (in_str != "") {
      # Inside a string: copy verbatim until matching quote.
      out = out c
      if (c == "\\" && i + 1 <= n) { i++; out = out substr(line, i, 1); i++; continue }
      if (c == in_str) { in_str = ""; i++; continue }
      i++; continue
    }
    # Outside strings + outside block comments
    if (c == "/" && substr(line, i + 1, 1) == "*") { in_block = 1; i += 2; continue }
    if (c == "/" && substr(line, i + 1, 1) == "/") { break }  # rest of line is comment
    if (c == "\"" || c == "'" || c == BT) {
      in_str = c
      out = out c
      i++
      continue
    }
    out = out c
    i++
  }
  print out
}
AWK_EOF

awk -f "$AWK_FILE" "$FILE" > "$STRIP_FILE"

# ---------------------------------------------------------------------------
# 1. Find the ALL_TOOLBAR_ITEMS array literal range.
#    Anchor: `const ALL_TOOLBAR_ITEMS` — then walk forward counting `[` / `]`
#    in the stripped source until depth hits 0. Inside the resulting range,
#    grep for `id: "…"` patterns.
# ---------------------------------------------------------------------------
start_line="$(grep -nE 'const[[:space:]]+ALL_TOOLBAR_ITEMS' "$STRIP_FILE" | head -1 | cut -d: -f1)"
if [[ -z "$start_line" ]]; then
  echo "ERROR: could not find 'const ALL_TOOLBAR_ITEMS' in $FILE" >&2
  exit 1
fi

end_line="$(awk -v s="$start_line" '
NR >= s {
  n = length($0)
  for (i = 1; i <= n; i++) {
    c = substr($0, i, 1)
    if (c == "[") { depth++; started = 1 }
    else if (c == "]") {
      depth--
      if (started && depth == 0) { print NR; exit }
    }
  }
}' "$STRIP_FILE")"

if [[ -z "$end_line" ]]; then
  echo "ERROR: could not find matching closing ']' for ALL_TOOLBAR_ITEMS" >&2
  exit 1
fi

# Declared id set: walk the array range char-by-char tracking object-nesting
# depth so a nested `{ ..., meta: { id: "inner" } }` does NOT count as a
# toolbar id. Real toolbar ids live at the first `{` level inside the
# outer `[ … ]` — i.e., when brace_depth (counting only `{`) is exactly 1
# relative to where the array opened.
declared="$(awk -v s="$start_line" -v e="$end_line" '
NR >= s && NR <= e {
  n = length($0)
  i = 1
  buf = ""
  while (i <= n) {
    c = substr($0, i, 1)
    if (c == "[") { bracket_depth++; i++; continue }
    if (c == "]") { bracket_depth--; i++; continue }
    if (c == "{") { brace_depth++; i++; continue }
    if (c == "}") { brace_depth--; i++; continue }
    # Only collect `id: "..."` when brace_depth == 1 (top-level entry object).
    # Accept either single or double quotes — project convention is double,
    # but ESLint/Prettier configs change and the cost of handling both is
    # one extra branch.
    if (brace_depth == 1) {
      if (substr($0, i, 3) == "id:") {
        j = i + 3
        while (substr($0, j, 1) == " " || substr($0, j, 1) == "\t") j++
        q = substr($0, j, 1)
        if (q == "\"" || q == "'\''") {
          k = j + 1
          val = ""
          while (k <= n && substr($0, k, 1) != q) {
            val = val substr($0, k, 1)
            k++
          }
          if (substr($0, k, 1) == q) {
            print val
            i = k + 1
            continue
          }
        }
      }
    }
    i++
  }
}' "$STRIP_FILE" | sort -u)"

# Referenced id set — linearize the stripped source so multi-line
# `toolbarItems.includes(\n  "foo"\n)` is matched as well as the single-line
# form. We replace newlines with spaces; the resulting big string is then
# pattern-matched. Dynamic args like `includes(item.id)` are skipped
# naturally — the regex requires a quoted literal.
referenced="$( { tr '\n' ' ' < "$STRIP_FILE" \
    | grep -oE 'toolbarItems\.includes\([[:space:]]*"[A-Za-z0-9_-]+"[[:space:]]*\)' \
    | grep -oE '"[A-Za-z0-9_-]+"' \
    | tr -d '"' ; \
  tr '\n' ' ' < "$STRIP_FILE" \
    | grep -oE "toolbarItems\\.includes\\([[:space:]]*'[A-Za-z0-9_-]+'[[:space:]]*\\)" \
    | grep -oE "'[A-Za-z0-9_-]+'" \
    | tr -d "'" ; } | sort -u)"

if [[ "$LIST" == true ]]; then
  echo "ALL_TOOLBAR_ITEMS array range: lines $start_line-$end_line"
  echo "Declared ids ($(echo "$declared" | grep -c .)):"
  echo "$declared" | sed 's/^/  /'
  echo "Referenced ids ($(echo "$referenced" | grep -c .)):"
  echo "$referenced" | sed 's/^/  /'
fi

# ---------------------------------------------------------------------------
# 2. Compare sets (bash 3.2 — no associative arrays, use comm).
# ---------------------------------------------------------------------------
dead="$(comm -23 <(echo "$declared") <(echo "$referenced"))"     # declared, not referenced
missing="$(comm -13 <(echo "$declared") <(echo "$referenced"))"  # referenced, not declared

failed=0
fail() { echo "FAIL: $*" >&2; failed=$((failed + 1)); }

if [[ -n "$dead" && "$dead" != "" ]]; then
  while IFS= read -r id; do
    [[ -z "$id" ]] && continue
    fail "id \"$id\" is in ALL_TOOLBAR_ITEMS but no toolbarItems.includes(\"$id\") gate references it — dead customizer entry"
  done <<< "$dead"
fi
if [[ -n "$missing" && "$missing" != "" ]]; then
  while IFS= read -r id; do
    [[ -z "$id" ]] && continue
    fail "id \"$id\" is gated by toolbarItems.includes(\"$id\") but not declared in ALL_TOOLBAR_ITEMS — customizer cannot toggle it"
  done <<< "$missing"
fi

if [[ "$failed" -eq 0 ]]; then
  d_count="$(echo "$declared" | grep -c .)"
  echo "OK: $d_count toolbar item(s), declared and referenced sets are in lockstep"
  exit 0
fi
exit 1
