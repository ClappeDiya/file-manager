#!/usr/bin/env bash
# check-tauri-invoke-types.sh — fail the gate when a `tauriInvoke(...)`
# call's return value is consumed (assigned, returned, used in an
# expression) without supplying an explicit `<T>` type argument.
#
# Why this exists:
#   `tauriInvoke<T>("name", args)` is the desktop app's IPC boundary.
#   When the type arg is supplied, TypeScript narrows the return to `T`
#   and downstream `.field` access is checked. When it is omitted, TS
#   infers `unknown`, and *every* downstream usage silently degrades —
#   `.field` compiles cleanly but yields runtime `undefined`. The only
#   compiler signal is "Object is of type 'unknown'", which contributors
#   often resolve via `as` casts that re-introduce the gap.
#
#   `await tauriInvoke("delete_X", { … })` STATEMENTS — where the return
#   is discarded — are legitimately untyped: there is nothing to narrow
#   and the implicit `unknown` is unused. This check distinguishes
#   the two cases:
#     * STATEMENT void use → ALLOWED untyped
#     * CONSUMED use (assigned / returned / used in expression) → FAIL
#       unless `<T>` is supplied.
#
#   Iter 5's `ipc-verify` already checks name-string drift between
#   frontend `tauriInvoke<T>("name")` callsites and Rust
#   `#[tauri::command]` handlers; this check is the orthogonal
#   "type-argument coverage" sibling. Same DNA as iters 5/9–33.
#
# What it flags:
#   * `<lvalue> = (await )? tauriInvoke("…", …)` without `<T>`
#   * `return (await )? tauriInvoke("…", …)` without `<T>`
#   * `tauriInvoke("…", …)` directly inside an expression position
#     (preceding non-whitespace, non-`await` token is `(`, `,`, `?`,
#     or `:`)
#
# What it ALLOWS:
#   * `(await )? tauriInvoke("…", …)` as a statement (preceding token
#     is newline, `;`, or `{`)
#   * Any `tauriInvoke<T>("…", …)` form — explicit type arg present
#   * `tauriInvokeSafe<T>(…)` — same shape, same rules
#
# Pure-bash + python3 (already required across iter 30/31/32/33).
# Sub-second.
#
# Usage:
#   ./scripts/check-tauri-invoke-types.sh         # verify; non-zero on findings
#   ./scripts/check-tauri-invoke-types.sh --list  # debug: print every call site categorized

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

LIST=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)  LIST=true; shift ;;
    -h|--help)
      sed -n '2,46p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "Unknown arg: $1 (try --help)"; exit 2 ;;
  esac
done

if ! command -v python3 >/dev/null 2>&1; then
  echo "ERROR: python3 not on PATH" >&2
  exit 1
fi

findings_file="$(mktemp -t check-tauri-invoke-types.XXXXXX)"
trap 'rm -f "$findings_file"' EXIT

python3 - "$LIST" >"$findings_file" <<'PY'
import os
import re
import sys
from pathlib import Path

list_mode = (sys.argv[1] == "true")

EXCLUDED = {"node_modules", ".next", ".vite", ".turbo", ".git", "target", "dist", "unified-file-ops"}
EXTS = {".ts", ".tsx"}

# Match any `tauriInvoke(...)` or `tauriInvokeSafe(...)` call. We capture
# the type-arg presence and the command name separately.
#   group 1: function name (`tauriInvoke` or `tauriInvokeSafe`)
#   group 2: type-arg block (e.g. `<Foo>`) or empty
#   group 3: command name (string-literal contents)
CALL_RE = re.compile(
    r'\b(tauriInvoke(?:Safe)?)(<[^>(){};]+>)?\(\s*"([^"]+)"',
)

# A "consumed" context: the call appears at a position where its return
# is used. We classify by walking back through whitespace + an optional
# `await` and inspecting the immediate preceding non-whitespace token.
CONSUMED_PREV_CHARS = set("=(,?:")

def walk_back_through_await(text: str, idx: int) -> int:
    """From idx (the start of `tauriInvoke...`), walk back through
    whitespace and an optional `await` keyword. Return the index of the
    immediate preceding non-whitespace char."""
    i = idx - 1
    while i >= 0 and text[i] in " \t":
        i -= 1
    # If we hit "await" (with surrounding whitespace already stripped),
    # consume it and keep walking.
    if i >= 4 and text[i - 4:i + 1] == "await":
        i -= 5
        while i >= 0 and text[i] in " \t":
            i -= 1
    return i

def categorize(text: str, call_start: int) -> str:
    i = walk_back_through_await(text, call_start)
    if i < 0:
        return "STATEMENT"
    prev = text[i]
    # `return tauriInvoke(...)` — walk back, look for the keyword `return`
    if prev.isalnum() or prev == "_":
        # Could be `return`, `const`, identifier, etc. Pull the token.
        j = i
        while j >= 0 and (text[j].isalnum() or text[j] == "_"):
            j -= 1
        tok = text[j + 1:i + 1]
        if tok == "return":
            return "CONSUMED:return"
        # Other identifier (`const x = await …`) handled by the `=` case below
        # — but if the line is just `<ident> tauriInvoke(...)` with no `=`,
        # that's syntactically invalid TypeScript so we don't worry about it.
        return "STATEMENT"
    if prev in CONSUMED_PREV_CHARS:
        return f"CONSUMED:{prev}"
    if prev == "\n" or prev == ";" or prev == "{" or prev == "}":
        return "STATEMENT"
    return "STATEMENT"

call_sites = []  # (file, line_no, fname, typed, category, line_text)
for dirpath, dirnames, filenames in os.walk("src"):
    dirnames[:] = [d for d in dirnames if d not in EXCLUDED]
    for fn in filenames:
        if Path(fn).suffix not in EXTS:
            continue
        p = Path(dirpath) / fn
        try:
            text = p.read_text(errors="ignore")
        except Exception:
            continue
        for m in CALL_RE.finditer(text):
            fname = m.group(1)
            typed = m.group(2) is not None
            line_no = text[:m.start()].count("\n") + 1
            line = text.splitlines()[line_no - 1] if line_no <= len(text.splitlines()) else ""
            cat = categorize(text, m.start())
            call_sites.append((str(p), line_no, fname, typed, cat, line.strip()))

# Findings: any CONSUMED:* category with typed=False is a violation.
violations = [s for s in call_sites if not s[3] and s[4].startswith("CONSUMED")]

if list_mode:
    by_cat = {}
    for s in call_sites:
        key = (
            "typed" if s[3]
            else ("consumed-untyped" if s[4].startswith("CONSUMED") else "void-untyped")
        )
        by_cat[key] = by_cat.get(key, 0) + 1
    print(f"INFO\ttotal={len(call_sites)}\ttyped={by_cat.get('typed', 0)}\tvoid-untyped={by_cat.get('void-untyped', 0)}\tconsumed-untyped={by_cat.get('consumed-untyped', 0)}")

for f, ln, fname, _, cat, line in violations:
    print(f"FINDING\t{f}\t{ln}\t{fname}\t{cat}\t{line[:120]}")

if not violations:
    print(f"OK\ttotal={len(call_sites)}\ttyped={sum(1 for s in call_sites if s[3])}\tvoid-untyped={sum(1 for s in call_sites if not s[3] and not s[4].startswith('CONSUMED'))}")
PY

# Parse python output. Emit summary + findings.
violations=()
info_line=""
ok_line=""
while IFS=$'\t' read -r tag rest; do
  case "$tag" in
    INFO)    info_line="$rest" ;;
    FINDING) violations+=("$rest") ;;
    OK)      ok_line="$rest" ;;
  esac
done <"$findings_file"

if [[ "$LIST" == true && -n "$info_line" ]]; then
  IFS=$'\t' read -ra parts <<<"$info_line"
  echo "tauriInvoke call sites: ${parts[*]}"
fi

if [[ "${#violations[@]}" -eq 0 ]]; then
  if [[ -n "$ok_line" ]]; then
    IFS=$'\t' read -ra parts <<<"$ok_line"
    echo "OK: ${parts[*]} (consumed-untyped: 0)"
  else
    echo "OK: tauriInvoke call sites clean"
  fi
  exit 0
fi

echo "FAIL: ${#violations[@]} tauriInvoke call site(s) consume the return value without a <T> type argument:" >&2
for v in "${violations[@]}"; do
  IFS=$'\t' read -r f ln fname cat line <<<"$v"
  echo "  $f:$ln  $fname  $cat" >&2
  echo "      $line" >&2
done
echo "      Fix: add a TypeScript type argument matching the Rust handler's return type, e.g." >&2
echo "      \`const x = await tauriInvoke<FooType>(\"foo_command\", { … });\`" >&2
echo "      Without <T>, TypeScript infers \`unknown\` and downstream .field access silently" >&2
echo "      yields runtime undefined." >&2
exit 1
