#!/usr/bin/env bash
# check-store-keys.sh — verify Zustand persist() storage keys are unique
# and follow the `ufop-*` namespace convention across src/stores/*.ts.
#
# Why this exists:
#   CLAUDE.md: "Stores using `persist()` middleware save to localStorage."
#   If two stores share a `name:` key, they silently overwrite each other's
#   persisted state. The failure mode is invisible at compile time, at
#   type-check time, and almost always at test time (tests reset store
#   state in `beforeEach`). It only hits real users — on the next reload,
#   their UI state is half from store A, half from store B, both corrupt.
#
#   This is the same DNA as the AppState wiring check: a contract not
#   enforced by the compiler that ruins runtime behaviour. Catch it at
#   the verify gate, in sub-second time, instead of in a bug report.
#
# What it checks:
#   1. For each src/stores/*.ts that calls `persist(`, find the `name:`
#      string literal inside its config object (the second arg).
#   2. All extracted names must be unique across the whole stores directory.
#   3. All extracted names must start with the `ufop-` prefix (project
#      convention — keeps us out of generic-name collision territory with
#      any third-party library that might also write to localStorage).
#
# Pre-processing:
#   - strips block comments (`/* ... */`, single- and multi-line)
#   - strips line comments (`//`, `///`)
# These two passes eliminate the obvious false-positive vector: a doc
# comment containing the substring `persist(` or `name: "ufop-foo"` would
# otherwise inflate the count or shadow the real key. Single/double-quoted
# string literals containing `name: "ufop-x"` are guarded against by
# anchoring the extractor on a `{` or `,` immediately before `name:` —
# real config props sit at object-property position, free text in a state
# field's string value does not.
#
# Pure-bash (3.2 compatible), no jq / node / python. Runs in well under a
# second. Mirrors the shape of check-appstate.sh / check-migrations.sh.
#
# Usage:
#   ./scripts/check-store-keys.sh           # verify; non-zero on duplicate/bad
#   ./scripts/check-store-keys.sh --list    # debug: print parsed name → file

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

STORES_DIR="src/stores"
PREFIX="ufop-"

LIST=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --list) LIST=true; shift ;;
    -h|--help)
      sed -n '2,33p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "Unknown arg: $1 (try --help)"; exit 2 ;;
  esac
done

if [[ ! -d "$STORES_DIR" ]]; then
  echo "ERROR: $STORES_DIR not found" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Collect (name, file) pairs.
# ---------------------------------------------------------------------------
# Project convention: one store per file, one `persist(...)` call per store.
# So if a file contains `persist(`, the file MUST contain exactly one
# `name: "..."` string-literal property (which is the persist config key).
# Anchoring at file granularity avoids the brittle "scan N lines after
# persist(" approach — Zustand state initializers can be hundreds of lines,
# so a fixed lookahead window either misses real names or scoops up
# unrelated `name:` properties from deep inside the state shape.
#
# Pre-processing strips:
#   1. `//`-style line comments (so commented-out persist calls don't count)
#   2. nothing else — block comments and JSX strings aren't relevant here
names=()        # parallel arrays — bash 3.2 has no associative arrays
files=()

# parse_store: state-machine tokenizer in awk that handles TypeScript
# block comments, line comments, single/double/backtick string literals
# (with escape sequences), and counts top-level `persist(` calls.
#
# Output (tab-separated, one line):
#   <persist_count>\t<name_or_marker>\t<persist_anchor_line>
#
# where <name_or_marker> is the extracted persist config name, or `<missing>`
# if no `name: "..."` property exists in the file at object-property position
# (i.e. preceded by `{` or `,`, not nested inside a string literal).
#
# The awk script tracks three pieces of state:
#   in_string  — current string-opening quote char (", ', or `), or empty
#   in_block   — 1 when inside `/* ... */`
#   buf        — rolling small buffer of the last few non-comment,
#                non-string characters, used to detect `[{,]<ws>name:`
# Once we see that anchor, the very next string literal we open is the
# persist key — we capture it and emit. This makes string-literal poisoning
# (e.g. `description: '{ name: "ufop-fake" }'`) literally unreachable: the
# inner quotes are skipped over while still inside the outer string.
AWK_FILE="$(mktemp -t check-store-keys.XXXXXX.awk)"
trap 'rm -f "$AWK_FILE"' EXIT
cat > "$AWK_FILE" <<'AWK_EOF'
BEGIN {
  in_block = 0
  in_string = ""
  buf = ""
  capturing = 0
  cap_value = ""
  name_emitted = ""
  persist_count = 0
  persist_line = 0
  ident_buf = ""
  BT = sprintf("%c", 96)  # backtick — written this way to avoid bash
                          # command-substitution issues when this awk
                          # body is embedded in $(cat <<EOF ... EOF).
  # Paren/brace context tracking so `name:` only matches at the *right*
  # nesting level inside the persist config object. Without this, a
  # nested object literal like `meta: { name: "x" }` in the state shape
  # would shadow the real persist key.
  persist_paren_depth = 0    # parens INSIDE the persist(...) call; 1 = just inside, 0 = outside
  in_config_arg = 0          # 1 once we've crossed the top-level `,` separating state from config
  config_brace_depth = 0     # braces INSIDE the config object; 1 = top level of config, >1 = nested
}
function is_ident_char(c) { return (c ~ /[a-zA-Z0-9_$]/) }
{
  s = $0
  n = length(s)
  in_line_comment = 0
  lineno = NR
  for (i = 1; i <= n; i++) {
    c = substr(s, i, 1)
    if (in_line_comment) break
    if (in_block) {
      if (c == "*" && i < n && substr(s, i+1, 1) == "/") { in_block = 0; i++ }
      continue
    }
    if (in_string != "") {
      if (c == "\\" && i < n) {
        if (capturing == 2) cap_value = cap_value substr(s, i+1, 1)
        i++; continue
      }
      if (c == in_string) {
        if (capturing == 2) {
          if (name_emitted == "") name_emitted = cap_value
          capturing = 0
        }
        in_string = ""
        continue
      }
      if (capturing == 2) cap_value = cap_value c
      continue
    }
    if (c == "/" && i < n) {
      nc = substr(s, i+1, 1)
      if (nc == "/") { in_line_comment = 1; break }
      if (nc == "*") { in_block = 1; i++; continue }
    }
    if (c == "\"" || c == "'" || c == BT) {
      in_string = c
      if (capturing == 1) { capturing = 2; cap_value = "" }
      ident_buf = ""
      buf = buf c
      if (length(buf) > 20) buf = substr(buf, length(buf) - 19)
      continue
    }
    if (capturing == 1 && c != " " && c != "\t") capturing = 0
    if (is_ident_char(c)) {
      ident_buf = ident_buf c
    } else {
      if (ident_buf == "persist" && c == "(") {
        persist_count++
        if (persist_line == 0) persist_line = lineno
        persist_paren_depth = 1
        in_config_arg = 0
        config_brace_depth = 0
        ident_buf = ""
        # The `(` itself is consumed by this branch — don't double-count below.
        buf = buf c
        if (length(buf) > 20) buf = substr(buf, length(buf) - 19)
        continue
      }
      ident_buf = ""
    }

    # Update paren / brace / comma context (only when not in string/comment).
    if (persist_paren_depth > 0) {
      if (c == "(") {
        persist_paren_depth++
      } else if (c == ")") {
        persist_paren_depth--
        if (persist_paren_depth == 0) {
          in_config_arg = 0
          config_brace_depth = 0
        }
      } else if (c == "," && persist_paren_depth == 1) {
        # Top-level comma inside persist() — separator between state arg
        # (first) and config arg (second). First top-level comma flips us
        # into config arg.
        if (!in_config_arg) in_config_arg = 1
      } else if (c == "{") {
        if (in_config_arg) config_brace_depth++
      } else if (c == "}") {
        if (in_config_arg && config_brace_depth > 0) config_brace_depth--
      }
    }

    buf = buf c
    if (length(buf) > 20) buf = substr(buf, length(buf) - 19)

    # Only arm name-capture when we are inside the persist config object
    # at its top level — never inside a nested object literal.
    if (in_config_arg && config_brace_depth == 1 && \
        match(buf, /[{,][ \t]*name:[ \t]*$/) > 0) {
      if (name_emitted == "") capturing = 1
    }
  }
  buf = buf " "
  if (length(buf) > 20) buf = substr(buf, length(buf) - 19)
}
END {
  out = (name_emitted == "" ? "<missing>" : name_emitted)
  printf "%d\t%s\t%d\n", persist_count, out, persist_line
}
AWK_EOF

parse_store() {
  awk -f "$AWK_FILE" "$@"
}

shopt -s nullglob
for ts in "$STORES_DIR"/*.ts; do
  parsed="$(parse_store "$ts")"
  pcount="$(printf '%s' "$parsed" | cut -f1)"
  pname="$(printf '%s' "$parsed" | cut -f2)"
  pline="$(printf '%s' "$parsed" | cut -f3)"

  [[ "$pcount" -eq 0 ]] && continue
  if [[ "$pcount" -gt 1 ]]; then
    echo "FAIL: $ts has $pcount persist() calls — convention is one persist() per store file (split the file or extract the second store)" >&2
    exit 1
  fi

  names+=("$pname")
  files+=("$ts:$pline")
done
shopt -u nullglob

count="${#names[@]}"

if [[ "$LIST" == true ]]; then
  echo "persist() call sites found: $count"
  for i in "${!names[@]}"; do
    printf '  %-30s  %s\n' "${names[$i]}" "${files[$i]}"
  done
fi

if [[ "$count" -eq 0 ]]; then
  # No persist() calls at all — not a failure, but worth noting in --list.
  [[ "$LIST" == true ]] && echo "(no persist() call sites in $STORES_DIR)"
  echo "OK: no persist() call sites in $STORES_DIR"
  exit 0
fi

# ---------------------------------------------------------------------------
# Duplicate detection: O(n^2) is fine for n < 50.
# ---------------------------------------------------------------------------
failed=0
fail() { echo "FAIL: $*" >&2; failed=$((failed + 1)); }

reported_dupes=""
for i in "${!names[@]}"; do
  ni="${names[$i]}"
  [[ "$ni" == "<missing>" ]] && continue
  # Skip if we already reported this duplicate.
  case " $reported_dupes " in *" $ni "*) continue ;; esac

  hits=""
  for j in "${!names[@]}"; do
    [[ "${names[$j]}" == "$ni" ]] && hits="$hits ${files[$j]}"
  done
  hit_count="$(printf '%s\n' "$hits" | wc -w | tr -d ' ')"
  if [[ "$hit_count" -gt 1 ]]; then
    fail "duplicate persist key \"$ni\" used by:$hits — stores will overwrite each other's localStorage"
    reported_dupes="$reported_dupes $ni"
  fi
done

# ---------------------------------------------------------------------------
# Missing name detection.
# ---------------------------------------------------------------------------
for i in "${!names[@]}"; do
  if [[ "${names[$i]}" == "<missing>" ]]; then
    fail "persist() at ${files[$i]} has no \`name:\` config — zustand will use a default key that may collide"
  fi
done

# ---------------------------------------------------------------------------
# Prefix convention.
# ---------------------------------------------------------------------------
for i in "${!names[@]}"; do
  ni="${names[$i]}"
  [[ "$ni" == "<missing>" ]] && continue
  if [[ "$ni" != "$PREFIX"* ]]; then
    fail "persist key \"$ni\" at ${files[$i]} does not start with \"$PREFIX\" — convention is ufop-* to avoid collisions with third-party libs"
  fi
done

# ---------------------------------------------------------------------------
# Report
# ---------------------------------------------------------------------------
if [[ "$failed" -eq 0 ]]; then
  unique_count="$(printf '%s\n' "${names[@]}" | sort -u | wc -l | tr -d ' ')"
  echo "OK: $count persist() call site(s), $unique_count unique key(s), all \"${PREFIX}*\" namespaced"
  exit 0
fi
exit 1
