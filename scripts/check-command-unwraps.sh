#!/usr/bin/env bash
# check-command-unwraps.sh — flag any panic-prone construct inside the body
# of a `#[tauri::command]` function (outside of `#[cfg(test)]` blocks).
# Originally flagged only `.unwrap()` / `.expect(`; iter 29 extended scope
# to the panic-macro family that has identical IPC blast radius.
#
# Why this exists:
#   CLAUDE.md states: "All async Rust errors return `Result<T, AppError>` —
#   never use `.unwrap()` in command handlers". Any construct that panics
#   inside a Tauri command crashes the host runtime — the frontend gets
#   an opaque IPC error and the user's session is poisoned for that
#   command's whole subsystem. The compiler can't catch this. Same DNA
#   as iter 5/9–28: a stated invariant the type system does not enforce.
#
#   Currently the codebase has zero violations in command bodies (all
#   unwraps/expects/panics in commands/ are inside #[cfg(test)] modules
#   where panic-on-fail is the idiomatic test assertion). The point of
#   this check is to LOCK IN that discipline — any future regression is
#   caught at the verify gate.
#
# What it flags (inside a `#[tauri::command]` body, outside `#[cfg(test)]`):
#   * `.unwrap()` — including `\n.unwrap()` (multi-line method chain)
#   * `.expect(` — including `.expect("msg")` form
#   * `panic!(`        (iter 29)
#   * `unreachable!(`  (iter 29 — "this code path is impossible" panics)
#   * `todo!(`         (iter 29 — should never reach prod in a command)
#   * `unimplemented!(` (iter 29 — same as todo!)
#
# What it ALLOWS:
#   * `.unwrap_or(...)`, `.unwrap_or_default()`, `.unwrap_or_else(...)` —
#     these do NOT panic.
#   * `debug_assert!()`, `assert!()` — invariant checks; intentional and
#     idiomatic. The "obvious-error" macros above are different: they
#     declare "this state should never occur" but ship anyway.
#   * Anything inside `#[cfg(test)]` — unwrap/panic is idiomatic for test
#     setup.
#   * Anything in non-command helper functions (those are not on the IPC
#     boundary; their panic propagates to a caller that may handle it).
#   * Doc comments, line comments, block comments, string literals — same
#     defensive tokenization as iter 10/11.
#
# Limitations (intentional):
#   * We only inspect `src-tauri/src/commands/*.rs`. Engine modules can use
#     unwrap; their callers (commands) are required to return Result.
#   * We do NOT walk `mod inner { ... }` recursively. In practice all
#     `#[tauri::command]` functions live at module top level.
#
# Pure-bash + awk, no jq / python. Sub-second.
#
# Usage:
#   ./scripts/check-command-unwraps.sh         # verify; non-zero on findings
#   ./scripts/check-command-unwraps.sh --list  # debug: list commands scanned

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CMD_DIR="src-tauri/src/commands"

LIST=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)  LIST=true; shift ;;
    -h|--help)
      sed -n '2,53p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "Unknown arg: $1 (try --help)"; exit 2 ;;
  esac
done

if [[ ! -d "$CMD_DIR" ]]; then
  echo "ERROR: commands directory not found: $CMD_DIR" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Combined tokenizer + scope tracker.
#
# Walks the file character-by-character:
#   * Strips block comments, line comments, char/string/raw-string bodies
#     (same lessons as iter 10/11).
#   * Tracks brace depth at the top level.
#   * When `#[cfg(test)]` is seen at depth 0, the next `{...}` block at any
#     depth that opens is recorded as a "test_zone" — its full span is
#     excluded from command-body scanning.
#   * When `#[tauri::command]` is seen at depth 0, the next `{...}` block
#     opening after the attribute and the function signature is recorded
#     as a "cmd_zone" — its body is scanned for forbidden calls.
#   * If a cmd_zone is wholly inside a test_zone, it is skipped.
#   * Within a cmd_zone, an offending call is `.unwrap()` (NOT followed by
#     `_or`, since `.unwrap_or(...)` is a distinct method), `.expect(`, or
#     a word-bounded panic-macro: `panic!(`, `unreachable!(`, `todo!(`,
#     `unimplemented!(`.
#
# Emits one line per violation:
#   FILE:LINE:COL  COMMAND_NAME  KIND  EXCERPT
# Where KIND ∈ { unwrap, expect, panic, unreachable, todo, unimplemented }.
# ---------------------------------------------------------------------------
AWK_FILE="$(mktemp -t check-command-unwraps.XXXXXX.awk)"
trap 'rm -f "$AWK_FILE"' EXIT

cat > "$AWK_FILE" <<'AWK_EOF'
BEGIN {
  filename = ""
}
FNR == 1 {
  filename = FILENAME
  reset_state()
}
{
  scan_line($0, FNR)
}
END {
  # Pending command never had a body emitted? Likely a parse mismatch but
  # there is nothing actionable — silent.
}

function reset_state(  i) {
  in_block = 0
  in_str = ""   # "", "'", "r<N>"
  depth = 0
  # Pending attribute: "test" or "cmd" or ""
  pending = ""
  # Stack of zone records: each entry is "kind:open_depth:start_line:cmd_name"
  # We index by depth at which it was opened.
  zone_count = 0
  delete zone_kind
  delete zone_open_depth
  delete zone_start_line
  delete zone_name
  # Pending command name (filled when we parse `fn <name>(` after #[tauri::command])
  pending_name = ""
}

# Return 1 if any active zone is of kind "test"
function in_any_test_zone(   i) {
  for (i = 1; i <= zone_count; i++) {
    if (zone_kind[i] == "test") return 1
  }
  return 0
}

# Return innermost active command zone name, or "" if none
function active_cmd_name(   i, name) {
  name = ""
  for (i = 1; i <= zone_count; i++) {
    if (zone_kind[i] == "cmd") name = zone_name[i]
  }
  return name
}

function scan_line(line, lineno,    exe, name, col, excerpt, sig, parts, n2, i) {
  # Pass 1: tokenize source line into an "executable view" with all
  # comments and string contents replaced by spaces. String/block state
  # is carried across lines via in_str / in_block.
  exe = tokenize_line(line)

  # Pass 2: attribute detection (must run BEFORE brace walk so the next
  # `{` we encounter opens the correct kind of zone).
  if (exe ~ /#\[cfg\(test\)\]/ || exe ~ /#\[cfg\(any\([^)]*test[^)]*\)\)\]/) {
    pending = "test"
    pending_name = ""
  }
  if (exe ~ /#\[tauri::command/) {
    pending = "cmd"
    pending_name = ""
  }

  # Pass 3: fn-name detection (eager — runs same line as the `fn name(`).
  if (pending == "cmd" && pending_name == "") {
    if (match(exe, /(pub[[:space:]]+(\([^)]+\)[[:space:]]+)?)?(async[[:space:]]+)?fn[[:space:]]+[A-Za-z_][A-Za-z0-9_]*/)) {
      sig = substr(exe, RSTART, RLENGTH)
      n2 = split(sig, parts, /[[:space:]]+/)
      pending_name = parts[n2]
    }
  }

  # Pass 4: walk the executable view char-by-char, managing the zone stack
  # AND scanning for forbidden calls inline. Inline scanning is necessary
  # because a one-liner like `fn foo() { Some(1).unwrap() }` opens and
  # closes its cmd zone within the same line — a post-walk scan would see
  # active_cmd_name() == "" by the time it ran.
  walk_and_scan(exe, lineno)
}

# Walk the stripped line: manage zone stack on { / } AND flag .unwrap() /
# .expect( while inside an active cmd zone (not nested in a test zone).
# By doing both in one pass we correctly handle bodies that begin AND end
# on the same line.
function walk_and_scan(exe, lineno,    i, n, c, name, excerpt, prev) {
  n = length(exe)
  i = 1
  while (i <= n) {
    c = substr(exe, i, 1)

    # Forbidden-call detection at THIS position (only when inside a cmd
    # zone that is not itself within a test zone). `.unwrap()` is exactly
    # 9 chars; `.unwrap_or...` starts `.unwrap_` so the literal-substring
    # comparison naturally excludes the safe variants. Panic-macro names
    # are matched on a word boundary so `mypanic!` doesn't trip.
    name = active_cmd_name()
    if (name != "" && !in_any_test_zone()) {
      if (substr(exe, i, 9) == ".unwrap()") {
        excerpt = trim(exe)
        printf "%s:%d:%d\t%s\tunwrap\t%s\n", filename, lineno, i, name, excerpt
      } else if (substr(exe, i, 8) == ".expect(") {
        excerpt = trim(exe)
        printf "%s:%d:%d\t%s\texpect\t%s\n", filename, lineno, i, name, excerpt
      } else {
        # Panic-macro family. Require word boundary on the left (so
        # tokens like `mypanic!` or `_panic!` don't match).
        prev = (i == 1 ? " " : substr(exe, i - 1, 1))
        if (prev !~ /[A-Za-z0-9_]/) {
          if (substr(exe, i, 7) == "panic!(") {
            excerpt = trim(exe)
            printf "%s:%d:%d\t%s\tpanic\t%s\n", filename, lineno, i, name, excerpt
          } else if (substr(exe, i, 13) == "unreachable!(") {
            excerpt = trim(exe)
            printf "%s:%d:%d\t%s\tunreachable\t%s\n", filename, lineno, i, name, excerpt
          } else if (substr(exe, i, 6) == "todo!(") {
            excerpt = trim(exe)
            printf "%s:%d:%d\t%s\ttodo\t%s\n", filename, lineno, i, name, excerpt
          } else if (substr(exe, i, 15) == "unimplemented!(") {
            excerpt = trim(exe)
            printf "%s:%d:%d\t%s\tunimplemented\t%s\n", filename, lineno, i, name, excerpt
          }
        }
      }
    }

    if (c == "{") {
      if (pending != "") {
        zone_count++
        zone_kind[zone_count] = pending
        zone_open_depth[zone_count] = depth
        zone_start_line[zone_count] = lineno
        zone_name[zone_count] = (pending == "cmd" ? pending_name : "")
        pending = ""
        pending_name = ""
      }
      depth++
    } else if (c == "}") {
      depth--
      while (zone_count > 0 && zone_open_depth[zone_count] >= depth) {
        zone_count--
      }
    }
    i++
  }
}

# Tokenizer: walk the raw line char-by-char, carry in_str/in_block across
# calls, return the executable view (strings/comments replaced by spaces).
function tokenize_line(line,    n, i, c, exe, prev, head3, j, h, k, end, rest) {
  n = length(line)
  i = 1
  exe = ""
  while (i <= n) {
    c = substr(line, i, 1)
    if (in_block) {
      if (c == "*" && substr(line, i + 1, 1) == "/") { in_block = 0; i += 2; exe = exe "  "; continue }
      i++; exe = exe " "; continue
    }
    if (in_str == "\"") {
      if (c == "\\" && i + 1 <= n) { i += 2; exe = exe "  "; continue }
      if (c == "\"") { in_str = ""; exe = exe "\""; i++; continue }
      i++; exe = exe " "; continue
    }
    if (in_str == "'") {
      if (c == "\\" && i + 1 <= n) { i += 2; exe = exe "  "; continue }
      if (c == "'") { in_str = ""; exe = exe "'"; i++; continue }
      i++; exe = exe " "; continue
    }
    if (substr(in_str, 1, 1) == "r") {
      h = substr(in_str, 2) + 0
      if (c == "\"") {
        end = ""
        for (k = 1; k <= h; k++) end = end "#"
        if (substr(line, i + 1, h) == end) {
          in_str = ""
          exe = exe "\"" end
          i += 1 + h
          continue
        }
      }
      i++; exe = exe " "; continue
    }
    if (c == "/" && substr(line, i + 1, 1) == "*") { in_block = 1; i += 2; exe = exe "  "; continue }
    if (c == "/" && substr(line, i + 1, 1) == "/") {
      while (i <= n) { exe = exe " "; i++ }
      break
    }
    prev = (i == 1 ? " " : substr(line, i - 1, 1))
    if (prev !~ /[A-Za-z0-9_]/ && (c == "r" || c == "b")) {
      head3 = substr(line, i, 3)
      if (head3 ~ /^br[#"]/) {
        j = i + 2
        h = 0
        while (substr(line, j, 1) == "#") { h++; j++ }
        if (substr(line, j, 1) == "\"") {
          in_str = "r" h
          exe = exe "br"
          for (k = 0; k < h; k++) exe = exe "#"
          exe = exe "\""
          i = j + 1
          continue
        }
      }
      if (c == "r") {
        j = i + 1
        h = 0
        while (substr(line, j, 1) == "#") { h++; j++ }
        if (substr(line, j, 1) == "\"") {
          in_str = "r" h
          exe = exe "r"
          for (k = 0; k < h; k++) exe = exe "#"
          exe = exe "\""
          i = j + 1
          continue
        }
      }
      if (c == "b" && substr(line, i + 1, 1) == "\"") {
        in_str = "\""
        exe = exe "b\""
        i += 2
        continue
      }
    }
    if (c == "\"") { in_str = "\""; exe = exe "\""; i++; continue }
    if (c == "'") {
      rest = substr(line, i + 1)
      if (rest ~ /^[A-Za-z_][A-Za-z0-9_]*[^'A-Za-z0-9_\\]/ \
          || rest ~ /^[A-Za-z_][A-Za-z0-9_]*$/) {
        exe = exe "'"; i++; continue
      }
      in_str = "'"; exe = exe "'"; i++; continue
    }
    exe = exe c
    i++
  }
  return exe
}

function trim(s) {
  sub(/^[[:space:]]+/, "", s)
  sub(/[[:space:]]+$/, "", s)
  return s
}
AWK_EOF

# ---------------------------------------------------------------------------
# Run scanner across every commands/*.rs (excluding mod.rs).
# Collect findings, count, and pretty-print.
# ---------------------------------------------------------------------------
findings_file="$(mktemp -t check-command-unwraps-findings.XXXXXX)"
trap 'rm -f "$AWK_FILE" "$findings_file"' EXIT

shopt -s nullglob 2>/dev/null || true
files=()
for f in "$CMD_DIR"/*.rs; do
  base="$(basename "$f")"
  [[ "$base" == "mod.rs" ]] && continue
  files+=("$f")
done

if [[ "${#files[@]}" -eq 0 ]]; then
  echo "ERROR: no command files found under $CMD_DIR" >&2
  exit 1
fi

awk -f "$AWK_FILE" "${files[@]}" > "$findings_file" 2>/dev/null || true

if [[ "$LIST" == true ]]; then
  echo "Scanned ${#files[@]} file(s) under $CMD_DIR"
  cmd_count="$(grep -h '#\[tauri::command' "${files[@]}" 2>/dev/null | wc -l | tr -d ' ')"
  echo "Detected $cmd_count #[tauri::command] attribute(s)"
fi

count="$(wc -l < "$findings_file" | tr -d ' ')"

if [[ "$count" -eq 0 ]]; then
  echo "OK: ${#files[@]} command file(s) scanned, 0 panic-prone constructs in command bodies"
  exit 0
fi

echo "FAIL: $count panic-prone construct(s) inside a #[tauri::command] body" >&2
echo "      (kinds: unwrap, expect, panic!, unreachable!, todo!, unimplemented!)" >&2
echo "      (use .map_err(...)? or .ok_or_else(|| AppError::...)? instead — see CLAUDE.md)" >&2
while IFS=$'\t' read -r loc cmd kind excerpt; do
  echo "  $loc  fn $cmd ($kind)" >&2
  echo "      $excerpt" >&2
done < "$findings_file"
exit 1
