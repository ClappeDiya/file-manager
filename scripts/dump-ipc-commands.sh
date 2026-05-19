#!/usr/bin/env bash
# dump-ipc-commands.sh — regenerate docs/ipc-commands.md from
# `#[tauri::command]` declarations under src-tauri/src/commands/.
#
# The Tauri IPC surface (frontend ↔ Rust) is the actual public API of the
# desktop app, but it is spread across ~30 *_commands.rs modules and ~360
# functions. Without an index, frontend devs cannot tell whether a command
# already exists or has to be added. This script produces a single
# searchable markdown file from the source so it cannot drift.
#
# Usage:
#   ./scripts/dump-ipc-commands.sh                    # write to docs/ipc-commands.md
#   ./scripts/dump-ipc-commands.sh --check            # exit 1 if the file is stale
#   ./scripts/dump-ipc-commands.sh --stdout           # print to stdout, do not write
#   ./scripts/dump-ipc-commands.sh --verify           # run BOTH verify modes below
#   ./scripts/dump-ipc-commands.sh --verify-handler   # exit 1 if any #[tauri::command]
#                                                       is not registered in lib.rs
#                                                       invoke_handler! (Rust → wiring)
#   ./scripts/dump-ipc-commands.sh --verify-frontend  # exit 1 if any frontend
#                                                       tauriInvoke<T>("name") calls a
#                                                       command that does not exist
#                                                       (frontend → wiring)
#
# Re-run after adding/removing commands. The verify gate stays untouched on
# purpose — drift here is non-fatal, only documentation, so we don't make
# every contributor pay for it on every commit.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CMD_DIR="src-tauri/src/commands"
OUT="docs/ipc-commands.md"

MODE="write"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --check)             MODE="check"; shift ;;
    --stdout)            MODE="stdout"; shift ;;
    --verify)            MODE="verify"; shift ;;
    --verify-handler)    MODE="verify-handler"; shift ;;
    --verify-frontend)   MODE="verify-frontend"; shift ;;
    -h|--help) sed -n '2,28p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "Unknown arg: $1 (try --help)"; exit 2 ;;
  esac
done

if [[ ! -d "$CMD_DIR" ]]; then
  echo "ERROR: $CMD_DIR not found (run from project root)" >&2
  exit 1
fi

# Extract command names from one *_commands.rs file. A command is any
# `pub (async)? fn NAME(` line whose immediately preceding non-blank line is
# `#[tauri::command]` (or `#[tauri::command(...)]`).
extract_commands() {
  local f="$1" want_fn=0 trimmed rest name
  while IFS= read -r line; do
    if [[ "$line" == "#[tauri::command"* ]]; then
      want_fn=1
      continue
    fi
    if [[ $want_fn -eq 1 ]]; then
      trimmed="${line#"${line%%[![:space:]]*}"}"
      # Blank line or another attribute (e.g. #[allow(...)]) → keep waiting.
      if [[ -z "$trimmed" || "$trimmed" == "#["* ]]; then
        continue
      fi
      if [[ "$trimmed" == "pub async fn "* || "$trimmed" == "pub fn "* ]]; then
        rest="${trimmed#pub }"
        rest="${rest#async }"
        rest="${rest#fn }"
        name="${rest%%(*}"
        name="${name%% *}"
        name="${name%%<*}"
        [[ -n "$name" ]] && echo "$name"
      fi
      want_fn=0
    fi
  done < "$f"
}

# Same as extract_commands but emits "ATTR_LINE<tab>NAME" so verify mode can
# point at the exact #[tauri::command] line when reporting a missing handler.
extract_commands_with_line() {
  local f="$1" want_fn=0 trimmed rest name lineno=0 attr_line=0
  while IFS= read -r line; do
    lineno=$((lineno + 1))
    if [[ "$line" == "#[tauri::command"* ]]; then
      want_fn=1
      attr_line=$lineno
      continue
    fi
    if [[ $want_fn -eq 1 ]]; then
      trimmed="${line#"${line%%[![:space:]]*}"}"
      if [[ -z "$trimmed" || "$trimmed" == "#["* ]]; then
        continue
      fi
      if [[ "$trimmed" == "pub async fn "* || "$trimmed" == "pub fn "* ]]; then
        rest="${trimmed#pub }"
        rest="${rest#async }"
        rest="${rest#fn }"
        name="${rest%%(*}"
        name="${name%% *}"
        name="${name%%<*}"
        [[ -n "$name" ]] && printf '%s\t%s\n' "$attr_line" "$name"
      fi
      want_fn=0
    fi
  done < "$f"
}

# Extract every bare function name registered inside the
# `.invoke_handler(tauri::generate_handler![ ... ])` block of lib.rs. Lines
# inside the block look like `    module_commands::fn_name,`; comments and
# blank lines are ignored. The block ends at the first `])` line.
LIB_RS="src-tauri/src/lib.rs"
extract_handler_names() {
  local in_block=0 line trimmed name
  while IFS= read -r line; do
    if [[ $in_block -eq 0 ]]; then
      [[ "$line" == *".invoke_handler(tauri::generate_handler!["* ]] && in_block=1
      continue
    fi
    trimmed="${line#"${line%%[![:space:]]*}"}"
    [[ "$trimmed" == "])"* ]] && break
    [[ -z "$trimmed" || "$trimmed" == "//"* ]] && continue
    [[ "$trimmed" != *"::"* ]] && continue
    name="${trimmed##*::}"   # everything after last ::
    name="${name%%,*}"        # drop trailing comma + remainder
    name="${name%% *}"        # drop trailing whitespace + comment
    [[ -n "$name" ]] && printf '%s\n' "$name"
  done < "$LIB_RS"
}

# Extract `tauriInvoke<T>("name")` / `tauriInvokeSafe<T>("name")` callsites from
# one .ts/.tsx file. Emits "LINE<tab>NAME" pairs. Handles both inline form
# (`tauriInvoke<X>("foo", …)`) and multi-line form where the opening `(` is at
# the end of the line and the string literal sits on the next non-blank line.
# Dynamic dispatch (`tauriInvoke<X>(name, …)` with a variable) is silently
# skipped — we make no claim about it rather than reporting a false positive.
FRONTEND_SRC="src"
extract_invocations_with_line() {
  local f="$1" want_name=0 lineno=0 attr_line=0 trimmed
  # Type param uses `[^()]*` rather than `[^>]*` so the regex tolerates nested
  # generics (e.g. `tauriInvoke<Record<string, any>>(...)`,
  # `tauriInvokeSafe<Array<{ correlation_id: string }>>(...)`). TS type params
  # never contain `(`, so this is a safe boundary; the regex engine backtracks
  # across nested `>` to land on the real call's opening `(`.
  local re_inline='tauriInvoke(Safe)?<[^()]*>\([[:space:]]*"([A-Za-z_][A-Za-z0-9_]*)"'
  local re_multi='tauriInvoke(Safe)?<[^()]*>\([[:space:]]*$'
  local re_string='^"([A-Za-z_][A-Za-z0-9_]*)"'
  while IFS= read -r line; do
    lineno=$((lineno + 1))
    line="${line%$'\r'}"
    if [[ "$line" =~ $re_inline ]]; then
      printf '%s\t%s\n' "$lineno" "${BASH_REMATCH[2]}"
      want_name=0
      continue
    fi
    if [[ "$line" =~ $re_multi ]]; then
      want_name=1
      attr_line=$lineno
      continue
    fi
    if [[ $want_name -eq 1 ]]; then
      trimmed="${line#"${line%%[![:space:]]*}"}"
      if [[ "$trimmed" =~ $re_string ]]; then
        printf '%s\t%s\n' "$attr_line" "${BASH_REMATCH[1]}"
      fi
      want_name=0
    fi
  done < "$f"
}

render() {
  local module_count=0 total=0 f module names count
  echo "# Tauri IPC Commands"
  echo
  echo "> Auto-generated from \`src-tauri/src/commands/*.rs\` by"
  echo "> \`scripts/dump-ipc-commands.sh\`. **Do not edit by hand** —"
  echo "> re-run \`pnpm docs:ipc\` after changing the command surface."
  echo
  echo "This is the canonical list of every \`#[tauri::command]\` callable"
  echo "from the React/TypeScript frontend via"
  echo "\`tauriInvoke<T>(\"command_name\", { args })\` (see"
  echo "\`src/hooks/use-tauri.ts\`) or its fallback-safe variant"
  echo "\`tauriInvokeSafe<T>()\`."
  echo
  echo "Each command is registered in \`src-tauri/src/lib.rs\`'s \`invoke_handler\`"
  echo "macro; if you add a new one here, register it there too."
  echo

  for f in "$CMD_DIR"/*_commands.rs; do
    [[ -f "$f" ]] || continue
    module="$(basename "$f" .rs)"
    names="$(extract_commands "$f" | sort -u || true)"
    [[ -z "$names" ]] && continue
    count=$(printf '%s\n' "$names" | grep -c . || true)
    total=$(( total + count ))
    module_count=$(( module_count + 1 ))
    if [[ "$count" -eq 1 ]]; then
      echo "## \`$module\` (1 command)"
    else
      echo "## \`$module\` ($count commands)"
    fi
    echo
    while IFS= read -r name; do
      echo "- \`$name\`"
    done <<< "$names"
    echo
  done

  echo "---"
  echo
  echo "**Surface:** $total commands across $module_count modules."
}

verify_frontend() {
  if [[ ! -d "$FRONTEND_SRC" ]]; then
    echo "ERROR: $FRONTEND_SRC/ not found (run from project root)" >&2
    return 2
  fi

  local declared_names_file
  declared_names_file="$(mktemp)"

  local f
  for f in "$CMD_DIR"/*_commands.rs; do
    [[ -f "$f" ]] || continue
    extract_commands "$f"
  done | sort -u > "$declared_names_file"

  local total=0 unknown=0 ln nm
  while IFS= read -r -d '' f; do
    while IFS=$'\t' read -r ln nm; do
      [[ -z "$nm" ]] && continue
      total=$((total + 1))
      if ! grep -qxF "$nm" "$declared_names_file"; then
        printf 'UNKNOWN:  %s:%s  tauriInvoke<...>("%s")  (no matching #[tauri::command])\n' "$f" "$ln" "$nm"
        unknown=$((unknown + 1))
      fi
    done < <(extract_invocations_with_line "$f")
  done < <(find "$FRONTEND_SRC" -type f \( -name '*.ts' -o -name '*.tsx' \) -print0)

  rm -f "$declared_names_file"

  printf 'Summary: %s frontend tauriInvoke callsites scanned, %s unknown.\n' "$total" "$unknown"

  if [[ $unknown -gt 0 ]]; then
    echo "ERROR: frontend tauriInvoke calls reference commands that do not exist." >&2
    return 1
  fi
  echo "OK: every tauriInvoke<T>(\"name\") references a real #[tauri::command]"
  return 0
}

verify_handler() {
  if [[ ! -f "$LIB_RS" ]]; then
    echo "ERROR: $LIB_RS not found (run from project root)" >&2
    return 2
  fi

  local declared_file handler_file declared_names_file
  declared_file="$(mktemp)"
  handler_file="$(mktemp)"
  declared_names_file="$(mktemp)"

  local f ln nm
  for f in "$CMD_DIR"/*_commands.rs; do
    [[ -f "$f" ]] || continue
    while IFS=$'\t' read -r ln nm; do
      [[ -n "$nm" ]] && printf '%s\t%s\t%s\n' "$f" "$ln" "$nm" >> "$declared_file"
    done < <(extract_commands_with_line "$f")
  done

  extract_handler_names | sort -u > "$handler_file"
  cut -f3 "$declared_file" | sort -u > "$declared_names_file"

  local declared_count registered_count missing=0 orphan=0
  declared_count="$(wc -l < "$declared_file" | tr -d ' ')"
  registered_count="$(wc -l < "$handler_file" | tr -d ' ')"

  local path
  while IFS=$'\t' read -r path ln nm; do
    if ! grep -qxF "$nm" "$handler_file"; then
      printf 'MISSING:  %s:%s  %s  (declared but not in invoke_handler!)\n' "$path" "$ln" "$nm"
      missing=$((missing + 1))
    fi
  done < "$declared_file"

  while IFS= read -r nm; do
    [[ -z "$nm" ]] && continue
    if ! grep -qxF "$nm" "$declared_names_file"; then
      printf 'ORPHAN:   lib.rs invoke_handler!  %s  (registered but no matching #[tauri::command])\n' "$nm"
      orphan=$((orphan + 1))
    fi
  done < "$handler_file"

  rm -f "$declared_file" "$handler_file" "$declared_names_file"

  printf 'Summary: %s declared, %s registered, %s missing, %s orphan.\n' \
    "$declared_count" "$registered_count" "$missing" "$orphan"

  if [[ $missing -gt 0 || $orphan -gt 0 ]]; then
    echo "ERROR: invoke_handler! is out of sync with #[tauri::command] declarations." >&2
    return 1
  fi
  echo "OK: every #[tauri::command] is registered in lib.rs invoke_handler!"
  return 0
}

case "$MODE" in
  stdout)
    render
    ;;
  write)
    mkdir -p "$(dirname "$OUT")"
    render > "$OUT.tmp"
    mv "$OUT.tmp" "$OUT"
    echo "Wrote $OUT"
    ;;
  check)
    current="$(render)"
    if [[ ! -f "$OUT" ]]; then
      echo "ERROR: $OUT does not exist — run: pnpm docs:ipc" >&2
      exit 1
    fi
    if ! diff -q <(printf '%s\n' "$current") "$OUT" >/dev/null 2>&1; then
      echo "ERROR: $OUT is stale — run: pnpm docs:ipc" >&2
      diff -u "$OUT" <(printf '%s\n' "$current") | head -40 >&2 || true
      exit 1
    fi
    echo "$OUT is up to date"
    ;;
  verify-handler)
    verify_handler
    exit $?
    ;;
  verify-frontend)
    verify_frontend
    exit $?
    ;;
  verify)
    # Run both directions and return non-zero if either fails. Print a blank
    # line between sections so the two summaries are easy to scan.
    rc_total=0
    verify_handler || rc_total=$?
    echo
    verify_frontend || rc_total=$?
    exit "$rc_total"
    ;;
esac
