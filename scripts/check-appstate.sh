#!/usr/bin/env bash
# check-appstate.sh — verify every AppState field is wired into Tauri state.
#
# CLAUDE.md flags this as a footgun: "All managers are initialized in
# `initialize_app_state()`, stored in an `AppState` struct, then individually
# registered via `.manage()` on the Tauri builder." If a new field is added
# to `struct AppState { ... }` but the corresponding `.manage(app_state.X)`
# call is forgotten, the code still compiles (struct literals are checked,
# `.manage()` chain calls are not). At runtime, any command that tries to
# access the unmanaged state via `tauri::State<T>` panics.
#
# This script reads src-tauri/src/lib.rs and confirms:
#   1. Every field in `struct AppState { ... }` appears in a
#      `.manage(app_state.<field>)` call.
#   2. Every `.manage(app_state.<field>)` call references a real field
#      (catches stale registrations after a rename/remove).
#
# Usage:
#   ./scripts/check-appstate.sh           # verify (exit 1 if mismatched)
#   ./scripts/check-appstate.sh --list    # print the parsed field list

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

LIB_RS="src-tauri/src/lib.rs"

MODE="verify"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)    MODE="list"; shift ;;
    -h|--help) sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "Unknown arg: $1 (try --help)"; exit 2 ;;
  esac
done

if [[ ! -f "$LIB_RS" ]]; then
  echo "ERROR: $LIB_RS not found (run from project root)" >&2
  exit 1
fi

# Extract every field name from the `struct AppState { ... }` block. The block
# spans from `struct AppState {` until the matching `}` at column 0.
extract_appstate_fields() {
  local in_block=0 line trimmed
  local re_field='^([a-z_][a-z_0-9]*):[[:space:]]'
  while IFS= read -r line; do
    if [[ $in_block -eq 0 ]]; then
      [[ "$line" == "struct AppState {"* ]] && in_block=1
      continue
    fi
    [[ "$line" == "}"* ]] && break
    trimmed="${line#"${line%%[![:space:]]*}"}"
    [[ -z "$trimmed" || "$trimmed" == "//"* ]] && continue
    if [[ "$trimmed" =~ $re_field ]]; then
      printf '%s\n' "${BASH_REMATCH[1]}"
    fi
  done < "$LIB_RS"
}

# Extract every X from `.manage(app_state.X)` calls anywhere in lib.rs.
extract_managed_fields() {
  local line
  local re_manage='\.manage\(app_state\.([a-z_][a-z_0-9]*)\)'
  while IFS= read -r line; do
    if [[ "$line" =~ $re_manage ]]; then
      printf '%s\n' "${BASH_REMATCH[1]}"
    fi
  done < "$LIB_RS"
}

if [[ "$MODE" == "list" ]]; then
  echo "# AppState fields"
  extract_appstate_fields | sort -u
  echo
  echo "# .manage(app_state.*) calls"
  extract_managed_fields | sort -u
  exit 0
fi

fields_file="$(mktemp)"
managed_file="$(mktemp)"
trap 'rm -f "$fields_file" "$managed_file"' EXIT

extract_appstate_fields | sort -u > "$fields_file"
extract_managed_fields | sort -u > "$managed_file"

fcount="$(wc -l < "$fields_file" | tr -d ' ')"
mcount="$(wc -l < "$managed_file" | tr -d ' ')"

unmanaged=0
orphan=0

while IFS= read -r nm; do
  [[ -z "$nm" ]] && continue
  if ! grep -qxF "$nm" "$managed_file"; then
    printf 'UNMANAGED:  AppState.%s  (declared but never .manage()-d — commands using tauri::State<T> for this type will panic at runtime)\n' "$nm"
    unmanaged=$((unmanaged + 1))
  fi
done < "$fields_file"

while IFS= read -r nm; do
  [[ -z "$nm" ]] && continue
  if ! grep -qxF "$nm" "$fields_file"; then
    printf 'ORPHAN:     .manage(app_state.%s)  (no matching field in struct AppState)\n' "$nm"
    orphan=$((orphan + 1))
  fi
done < "$managed_file"

printf 'Summary: %s AppState fields, %s .manage() calls, %s unmanaged, %s orphan.\n' \
  "$fcount" "$mcount" "$unmanaged" "$orphan"

if [[ $unmanaged -gt 0 || $orphan -gt 0 ]]; then
  echo "ERROR: AppState fields are out of sync with .manage() registrations." >&2
  exit 1
fi
echo "OK: every AppState field is wired into Tauri state via .manage()"
