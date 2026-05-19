#!/usr/bin/env bash
# check-command-return-types.sh — every #[tauri::command] either returns
# `Result<T, AppError>` (canonical) or a bare `T` (infallible). Flag any
# `Result<T, X>` where X is not `AppError`.
#
# Why this exists:
#   CLAUDE.md: "All async Rust errors return `Result<T, AppError>`."
#   `AppError` carries the `{ message, advice }` fields that the frontend
#   renders into structured user-facing errors. A command returning
#   `Result<T, ()>` (opaque) or `Result<T, String>` (no advice field) or
#   `Result<T, Box<dyn Error>>` (stack traces leak) silently violates that
#   contract — it compiles cleanly and the frontend gets a degraded error
#   payload. Iter 14/29 covers panic discipline INSIDE commands; this
#   covers the AT-the-IPC-boundary contract.
#
#   Iter 30 introduced this check after a workspace survey found 2 real
#   violations (`safety_assess_intent`, `safety_confirm_intent` both
#   returning `Result<T, ()>`). Those were fixed in the same iteration;
#   this stage locks in the discipline going forward. Same DNA as iter
#   5/9–29: catch invariants the type system does not enforce.
#
# What it flags:
#   Any `#[tauri::command]` whose return type is `Result<...>` AND whose
#   error type is not `AppError`. Bare-T returns (no `->` or non-Result)
#   are allowed for genuinely infallible commands (e.g. `greet() -> String`).
#
# What it ALLOWS:
#   * No `->` clause (returns `()` implicitly — infallible).
#   * Bare return type that is not `Result<...>` (e.g. `String`,
#     `PlatformInfo`, `u32`).
#   * `Result<T, AppError>` (the canonical form).
#   * `Result<T, MyError>` where `MyError` happens to contain the
#     substring `AppError` (so e.g. `MyAppError` would pass — rare edge,
#     accepted false-negative).
#
# Pure-bash + awk. Sub-second on the ~350 commands in src-tauri/.
#
# Usage:
#   ./scripts/check-command-return-types.sh        # verify; non-zero on findings
#   ./scripts/check-command-return-types.sh --list # debug: list every command + return type

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CMD_DIR="src-tauri/src/commands"

LIST=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)  LIST=true; shift ;;
    -h|--help)
      sed -n '2,42p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "Unknown arg: $1 (try --help)"; exit 2 ;;
  esac
done

if [[ ! -d "$CMD_DIR" ]]; then
  echo "ERROR: $CMD_DIR not found" >&2
  exit 1
fi

shopt -s nullglob 2>/dev/null || true
files=()
for f in "$CMD_DIR"/*.rs; do
  [[ "$(basename "$f")" == "mod.rs" ]] && continue
  files+=("$f")
done

if [[ "${#files[@]}" -eq 0 ]]; then
  echo "ERROR: no command files found under $CMD_DIR" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Awk scanner: buffer lines after `#[tauri::command]` until the line that
# opens the function body (contains `{`). Parse the buffer for fn name +
# return type. Heuristic stop-on-`{` works because Rust struct literals
# never appear inside fn signature type annotations.
# ---------------------------------------------------------------------------
findings_file="$(mktemp -t check-command-return-types.XXXXXX)"
listing_file="$(mktemp -t check-command-return-types-listing.XXXXXX)"
trap 'rm -f "$findings_file" "$listing_file"' EXIT

awk '
BEGIN { in_sig = 0; sig = ""; cmd_start_line = 0; }
FNR == 1 { in_sig = 0; sig = ""; }
/^[[:space:]]*#\[tauri::command/ {
  in_sig = 1
  sig = $0
  cmd_start_line = FNR
  next
}
in_sig {
  sig = sig " " $0
  if (index($0, "{") > 0) {
    in_sig = 0
    # Parse fn name
    name = ""
    if (match(sig, /fn[[:space:]]+[A-Za-z_][A-Za-z0-9_]*/)) {
      fn_part = substr(sig, RSTART, RLENGTH)
      n = split(fn_part, parts, /[[:space:]]+/)
      name = parts[n]
    }
    # Parse return type: substring between "->" and the first "{"
    ret = ""
    arrow = index(sig, "->")
    if (arrow > 0) {
      after = substr(sig, arrow + 2)
      brace = index(after, "{")
      if (brace > 0) {
        ret = substr(after, 1, brace - 1)
        # trim
        sub(/^[[:space:]]+/, "", ret)
        sub(/[[:space:]]+$/, "", ret)
      }
    }
    # Emit listing for --list mode
    printf "%s:%d\t%s\t%s\n", FILENAME, cmd_start_line, name, (ret == "" ? "<no-return>" : ret) > "/dev/stderr"
    # Policy:
    # - no return clause (bare T implicit ()) → OK
    # - return is not Result<...> → bare T → OK
    # - return is Result<..., AppError> (substring match) → OK
    # - otherwise (Result<T, X> with X != AppError) → FAIL
    if (ret == "") next
    if (ret !~ /^Result[[:space:]]*</) next
    if (index(ret, "AppError") > 0) next
    printf "%s:%d\t%s\t%s\n", FILENAME, cmd_start_line, name, ret
  }
}
' "${files[@]}" 2>"$listing_file" >"$findings_file"

count="$(wc -l < "$findings_file" | tr -d ' ')"
total_cmds="$(wc -l < "$listing_file" | tr -d ' ')"

if [[ "$LIST" == true ]]; then
  echo "Scanned ${#files[@]} file(s); detected $total_cmds #[tauri::command] attributes"
  cat "$listing_file"
fi

if [[ "$count" -eq 0 ]]; then
  echo "OK: ${#files[@]} command file(s) scanned, $total_cmds #[tauri::command] return types — all canonical (Result<T, AppError> or bare T)"
  exit 0
fi

echo "FAIL: $count #[tauri::command](s) return Result<T, X> where X is not AppError:" >&2
while IFS=$'\t' read -r loc name ret; do
  echo "  $loc  fn $name -> $ret" >&2
done < "$findings_file"
echo "      CLAUDE.md: 'All async Rust errors return Result<T, AppError>.'" >&2
echo "      Fix: change the error type to AppError, OR change the signature to bare T if the command genuinely cannot fail." >&2
exit 1
