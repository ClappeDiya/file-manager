#!/usr/bin/env bash
# check-tauri-capabilities.sh — fail the gate when any sensitive Tauri
# capability ships unscoped (no `allow` block, or empty `allow` block).
#
# Why this exists:
#   Tauri 2's capability files (`src-tauri/capabilities/*.json`) declare
#   what the WebView is permitted to do. For the `fs:*` family in
#   particular, scope-less permissions like `"fs:allow-write"` (string
#   form) or `{ "identifier": "fs:allow-write", "allow": [] }` grant
#   filesystem access to the WHOLE host filesystem with no path
#   restriction. Iter 10's `check-tauri-config.sh` catches the obvious
#   "allow-all"/"withGlobalTauri" anti-patterns but does NOT enforce
#   scope coverage on the path-scoped permissions — every `fs:allow-*`
#   today is correctly scoped (see `src-tauri/capabilities/default.json`)
#   and this check LOCKS IN that discipline. Same DNA as iter 5/9–29:
#   stated invariant the type system / Tauri loader cannot see at gate
#   time.
#
# What it flags:
#   Any permission entry in any `src-tauri/capabilities/*.json` whose
#   `identifier` matches a sensitive prefix (currently `fs:`) AND whose
#   `allow` array is missing or empty. String-form permissions
#   (`"fs:allow-write"` without an object) are flagged too — they grant
#   the permission without any scope.
#
# What it ALLOWS:
#   * Non-scoped permission families (`core:*`, `dialog:*`, `os:*`,
#     `notification:*`, `shell:allow-open`, `updater:*`) — these are
#     either UI-level (dialog) or already-restricted-by-design (shell:
#     allow-open opens a URL, doesn't run arbitrary commands). String
#     form is canonical for them.
#   * Permissions with a non-empty `allow` array, regardless of what's
#     in it — the actual scope correctness is a separate concern (see
#     iter 10's `check-tauri-config.sh` for placeholder / over-broad
#     pattern detection).
#
# Pure-bash + Python's json module (already required by macOS / Linux
# system Python). Sub-second.
#
# Usage:
#   ./scripts/check-tauri-capabilities.sh         # verify; non-zero on unscoped sensitive permission
#   ./scripts/check-tauri-capabilities.sh --list  # debug: print every parsed permission

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CAP_DIR="src-tauri/capabilities"

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

if [[ ! -d "$CAP_DIR" ]]; then
  echo "ERROR: $CAP_DIR not found" >&2
  exit 1
fi

if ! command -v python3 >/dev/null 2>&1; then
  echo "ERROR: python3 not on PATH — cannot parse capability JSON" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Sensitive permission prefixes that REQUIRE a non-empty `allow` scope.
# Extend this list when new path-scoped permissions enter the project.
# ---------------------------------------------------------------------------
SENSITIVE_PREFIXES="fs:"

if [[ "$LIST" == true ]]; then
  echo "Scope-required prefixes: $SENSITIVE_PREFIXES"
fi

# ---------------------------------------------------------------------------
# Walk every capability JSON. Emit one line per finding:
#   <file>\t<identifier>\t<reason>
# ---------------------------------------------------------------------------
findings_file="$(mktemp -t check-tauri-capabilities.XXXXXX)"
trap 'rm -f "$findings_file"' EXIT

python3 - "$CAP_DIR" "$SENSITIVE_PREFIXES" "$LIST" >"$findings_file" <<'PY'
import json
import os
import sys

cap_dir, prefixes_raw, list_mode = sys.argv[1], sys.argv[2], sys.argv[3]
prefixes = [p for p in prefixes_raw.split() if p]
list_mode = (list_mode == "true")

findings = []
parsed = []

for fname in sorted(os.listdir(cap_dir)):
    if not fname.endswith(".json"):
        continue
    fpath = os.path.join(cap_dir, fname)
    try:
        data = json.loads(open(fpath).read())
    except Exception as e:
        findings.append((fpath, "<parse-error>", f"could not parse JSON: {e}"))
        continue
    for perm in data.get("permissions", []) or []:
        if isinstance(perm, str):
            ident = perm
            is_sensitive = any(ident.startswith(p) for p in prefixes)
            parsed.append((fpath, ident, "string-form", 0))
            if is_sensitive:
                findings.append((fpath, ident,
                    "string-form permission with a sensitive prefix — no `allow` scope means unrestricted access"))
        elif isinstance(perm, dict):
            ident = perm.get("identifier", "<no-id>")
            allow = perm.get("allow") or []
            is_sensitive = any(ident.startswith(p) for p in prefixes)
            parsed.append((fpath, ident, "object-form", len(allow)))
            if is_sensitive and len(allow) == 0:
                findings.append((fpath, ident,
                    "sensitive permission with empty or missing `allow` array — equivalent to unrestricted access"))
        else:
            findings.append((fpath, "<unknown>",
                f"unexpected permission shape (not string or object): {type(perm).__name__}"))

if list_mode:
    print("Parsed entries:", file=sys.stderr)
    for fpath, ident, kind, n in parsed:
        print(f"  {fpath}  {ident}  ({kind}, {n} scope entry(ies))", file=sys.stderr)

for fpath, ident, reason in findings:
    print(f"{fpath}\t{ident}\t{reason}")
PY

count="$(wc -l < "$findings_file" | tr -d ' ')"

if [[ "$count" -eq 0 ]]; then
  total="$(find "$CAP_DIR" -maxdepth 1 -name '*.json' | wc -l | tr -d ' ')"
  echo "OK: $total capability file(s) scanned, every sensitive permission carries a non-empty \`allow\` scope"
  exit 0
fi

echo "FAIL: $count sensitive permission(s) ship unscoped:" >&2
while IFS=$'\t' read -r fpath ident reason; do
  echo "      $fpath  $ident" >&2
  echo "        $reason" >&2
done <"$findings_file"
echo "      Fix: add an \`allow\` array with explicit path scopes, e.g.:" >&2
echo "        { \"identifier\": \"<id>\", \"allow\": [ { \"path\": \"\$HOME/**\" }, ... ] }" >&2
exit 1
