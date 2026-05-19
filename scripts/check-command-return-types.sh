#!/usr/bin/env bash
# check-command-return-types.sh — `#[tauri::command]` signature discipline.
# Currently enforces two invariants on every command signature:
#   1. Return type is `Result<T, AppError>` (canonical) or bare `T`
#      (infallible). Any `Result<T, X>` where X is not `AppError` fails.
#   2. Parameter names are snake_case (iter 34 extension).
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
#   Iter 30 introduced the return-type check after a workspace survey
#   found 2 real violations (`safety_assess_intent`, `safety_confirm_intent`
#   both returning `Result<T, ()>`). Those were fixed in the same
#   iteration; this stage locks in the discipline going forward. Iter 34
#   extended it with parameter naming discipline.
#
#   Rust's built-in `non_snake_case` lint already warns on camelCase
#   parameter names — but it can be locally disabled with
#   `#[allow(non_snake_case)]`. This check is the gate-level defense:
#   any camelCase parameter on a `#[tauri::command]` function fails the
#   gate regardless of lint state, because the IPC wire-format relies on
#   serde converting Rust snake_case → JS camelCase. A camelCase Rust
#   parameter would serialize as `someParam` on the wire AND `someParam`
#   in the Rust struct, leaking the implementation naming into the JSON
#   contract. Same DNA as iter 5/9–33.
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
    # Parse param list: substring between the fn-name `(` and the matching `)`.
    # Walk paren depth so nested parens in types (lifetimes, State<_, X>,
    # generic args with parens) do not trip the boundary.
    params_clause = ""
    paren_start = 0
    for (i = 1; i <= length(sig); i++) {
      c = substr(sig, i, 1)
      if (c == "(") { paren_start = i; break }
    }
    if (paren_start > 0) {
      depth = 0
      end = 0
      for (i = paren_start; i <= length(sig); i++) {
        c = substr(sig, i, 1)
        if (c == "(") depth++
        else if (c == ")") { depth--; if (depth == 0) { end = i; break } }
      }
      if (end > paren_start) {
        params_clause = substr(sig, paren_start + 1, end - paren_start - 1)
      }
    }
    # Parse return type: substring between "->" and the first "{".
    ret = ""
    arrow = index(sig, "->")
    if (arrow > 0) {
      after = substr(sig, arrow + 2)
      brace = index(after, "{")
      if (brace > 0) {
        ret = substr(after, 1, brace - 1)
        sub(/^[[:space:]]+/, "", ret)
        sub(/[[:space:]]+$/, "", ret)
      }
    }
    # Emit listing for --list mode
    printf "%s:%d\t%s\t%s\n", FILENAME, cmd_start_line, name, (ret == "" ? "<no-return>" : ret) > "/dev/stderr"

    # ── Return-type policy (iter 30) ──
    # - no return clause (bare T implicit ()) → OK
    # - return is not Result<...> → bare T → OK
    # - return is Result<..., AppError> (substring match) → OK
    # - otherwise (Result<T, X> with X != AppError) → FAIL
    if (ret != "" && ret ~ /^Result[[:space:]]*</ && index(ret, "AppError") == 0) {
      printf "%s:%d\t%s\tret-type\t%s\n", FILENAME, cmd_start_line, name, ret
    }

    # ── Parameter snake_case policy (iter 34) ──
    # Tokenize params_clause on top-level `,` (commas inside nested `<>` /
    # `()` are skipped). For each top-level param, capture the leading
    # identifier (before the first `:`) and check it is snake_case.
    if (params_clause != "") {
      # Split on top-level commas. We walk char-by-char tracking < / > and ( / )
      # nesting depth; emit a token whenever depth=0 and we hit `,` or end.
      token = ""
      ang = 0
      par = 0
      n2 = length(params_clause)
      for (i = 1; i <= n2; i++) {
        c = substr(params_clause, i, 1)
        if (c == "<") ang++
        else if (c == ">") ang--
        else if (c == "(") par++
        else if (c == ")") par--
        if (c == "," && ang == 0 && par == 0) {
          check_param_snake_case(token, FILENAME, cmd_start_line, name)
          token = ""
        } else {
          token = token c
        }
      }
      check_param_snake_case(token, FILENAME, cmd_start_line, name)
    }
  }
}

# Examine a single parameter token. Extract the param name (text before
# the first `:` outside attributes) and flag if it contains uppercase.
function check_param_snake_case(tok, file, line, fn,    s, attr_end, colon, ident) {
  s = tok
  # Trim
  sub(/^[[:space:]]+/, "", s)
  sub(/[[:space:]]+$/, "", s)
  if (s == "") return
  # Strip leading attributes like `#[allow(...)]`
  while (substr(s, 1, 2) == "#[") {
    attr_end = index(s, "]")
    if (attr_end == 0) return
    s = substr(s, attr_end + 1)
    sub(/^[[:space:]]+/, "", s)
  }
  # Strip `mut`
  if (substr(s, 1, 4) == "mut ") s = substr(s, 5)
  # `self`-receivers and reference params are skipped. Lifetimes only
  # appear in TYPE positions (after `:`), never as param names — no need
  # to special-case them here.
  if (s == "self") return
  if (substr(s, 1, 1) == "&") return
  # Take everything up to the first `:` as the param name (could have
  # nested patterns like `(a, b): (u32, u32)` — first token wins; pattern
  # destructuring is rare in command signatures)
  colon = index(s, ":")
  if (colon == 0) return  # no `:` — not a typed param
  ident = substr(s, 1, colon - 1)
  sub(/[[:space:]]+$/, "", ident)
  # Pattern destructuring → skip
  if (ident ~ /^\(/ || ident ~ /^\{/ || ident ~ /^\[/) return
  # Snake_case test: lowercase, digits, underscores only
  if (ident ~ /[A-Z]/) {
    printf "%s:%d\t%s\tparam-name\t%s\n", file, line, fn, ident
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
  echo "OK: ${#files[@]} command file(s) scanned, $total_cmds #[tauri::command] — return types canonical (Result<T, AppError> or bare T); all params snake_case"
  exit 0
fi

# Partition findings by kind.
ret_findings=()
param_findings=()
while IFS=$'\t' read -r loc name kind value; do
  case "$kind" in
    ret-type)   ret_findings+=("$loc|$name|$value") ;;
    param-name) param_findings+=("$loc|$name|$value") ;;
  esac
done < "$findings_file"

if [[ "${#ret_findings[@]}" -gt 0 ]]; then
  echo "FAIL: ${#ret_findings[@]} #[tauri::command](s) return Result<T, X> where X is not AppError:" >&2
  for r in "${ret_findings[@]}"; do
    IFS='|' read -r loc name ret <<<"$r"
    echo "  $loc  fn $name -> $ret" >&2
  done
  echo "      CLAUDE.md: 'All async Rust errors return Result<T, AppError>.'" >&2
  echo "      Fix: change the error type to AppError, OR change the signature to bare T if the command genuinely cannot fail." >&2
fi

if [[ "${#param_findings[@]}" -gt 0 ]]; then
  echo "FAIL: ${#param_findings[@]} #[tauri::command](s) declare camelCase parameter names:" >&2
  for p in "${param_findings[@]}"; do
    IFS='|' read -r loc name ident <<<"$p"
    echo "  $loc  fn $name(... $ident: ...)" >&2
  done
  echo "      Tauri serializes Rust parameter names directly into the JSON wire format." >&2
  echo "      Use snake_case in Rust (Tauri auto-converts to camelCase on the JS side via" >&2
  echo "      its default \`#[serde(rename_all = \"camelCase\")]\` handling)." >&2
  echo "      Fix: rename the parameter to snake_case." >&2
fi
exit 1
