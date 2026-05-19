#!/usr/bin/env bash
# check-tauri-versions.sh — verify every Tauri-related dependency uses
# the same MAJOR version across JS and Rust.
#
# Why this exists:
#   Tauri's JS layer (`@tauri-apps/api`, `@tauri-apps/cli`,
#   `@tauri-apps/plugin-*`) and its Rust layer (`tauri`, `tauri-build`,
#   `tauri-plugin-*`) share an IPC contract that is stable WITHIN a
#   major version but breaks across majors. Bumping `@tauri-apps/api` to
#   `^3.0.0` while leaving `tauri = "2"` in Cargo.toml leaves the project
#   compiling cleanly — the JS side serializes one wire format, the Rust
#   side expects another, and the only signal at runtime is opaque
#   "invalid invoke arguments" / "type mismatch" errors when any IPC
#   command is called. Same DNA as iter 5/9/10/11/12/13/14/15/16/17:
#   contract drift the type system does not enforce because the two
#   sides are in different ecosystems.
#
# What it checks:
#   * Every `@tauri-apps/*` entry in `package.json`, `admin/package.json`,
#     and `marketing/package.json` (admin/marketing are Next.js sites
#     today and have no Tauri deps; the check still scans in case that
#     changes).
#   * Every line in `src-tauri/Cargo.toml` matching `^tauri` (covers
#     `tauri`, `tauri-build`, and every `tauri-plugin-*`).
#   * All extracted major versions must be identical.
#
# Version-spec parsing accepts:
#     "^2.10.1"  →  2     (caret)
#     "~2.10.1"  →  2     (tilde)
#     "2.10.1"   →  2     (exact)
#     "2"        →  2     (bare)
#     "2.x"      →  2     (wildcard)
#     "^2.0.0-rc.1" → 2   (pre-release tag preserved by ignoring after `.`)
#     "*"        →  *     (any — treated as compatible-with-anything)
#     "workspace:*" → skip (pnpm workspace ref — different mechanism)
#
# Pure-bash + grep + sed, no jq / python. Sub-second.
#
# Usage:
#   ./scripts/check-tauri-versions.sh         # verify; non-zero on drift
#   ./scripts/check-tauri-versions.sh --list  # debug: print all parsed entries

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

LIST=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)  LIST=true; shift ;;
    -h|--help)
      sed -n '2,38p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "Unknown arg: $1 (try --help)"; exit 2 ;;
  esac
done

# ---------------------------------------------------------------------------
# extract_major <version-spec> — emit the major version digit, or "*" for
# wildcard, or empty for workspace:* / unrecognized.
# Strips a leading caret / tilde / equals / range char and takes the
# digits before the first `.` (or end-of-string for bare numbers).
# ---------------------------------------------------------------------------
extract_major() {
  local v="$1"
  # Skip workspace refs (pnpm-internal, not a real version)
  case "$v" in
    workspace:*|catalog:*|link:*|file:*|git+*|github:*) echo "SKIP"; return ;;
    "*"|x|X) echo "*"; return ;;
  esac
  # Strip leading range chars
  v="${v#^}"; v="${v#~}"; v="${v#=}"; v="${v#>}"; v="${v#<}"; v="${v#>=}"; v="${v#<=}"
  # Take digits before first . or end
  echo "$v" | sed -E 's/^([0-9]+).*/\1/'
}

# ---------------------------------------------------------------------------
# Collect all (source, name, version, major) tuples. Stored as parallel
# arrays for bash 3.2 compatibility.
# ---------------------------------------------------------------------------
src_arr=()
name_arr=()
ver_arr=()
maj_arr=()

# --- JS side: scan every workspace package.json (root + admin + marketing
#     + packages/*). `unified-file-ops/` is a stale snapshot — explicitly
#     excluded. node_modules / .next / .vite / .turbo / .git / target /
#     dist are pruned. Globbing tolerates missing dirs. ---
js_pkgs=()
while IFS= read -r p; do
  [[ -z "$p" ]] && continue
  js_pkgs+=("$p")
done < <(find . \
  \( -path '*/node_modules' -o -path '*/.next' -o -path '*/.vite' \
     -o -path '*/.turbo' -o -path '*/.git' -o -path '*/target' \
     -o -path '*/dist' -o -path '*/unified-file-ops' \) -prune -o \
  -type f -name 'package.json' -print 2>/dev/null \
  | sed -E 's|^\./||' \
  | sort)

for pkg in "${js_pkgs[@]}"; do
  [[ -f "$pkg" ]] || continue
  # Match `"@tauri-apps/...": "<spec>"` lines
  while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    # Extract name and version
    name="$(echo "$line" | sed -E 's/.*"(@tauri-apps\/[A-Za-z0-9_/-]+)".*/\1/')"
    ver="$(echo "$line"  | sed -E 's/.*"@tauri-apps\/[A-Za-z0-9_/-]+"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')"
    maj="$(extract_major "$ver")"
    src_arr+=("$pkg")
    name_arr+=("$name")
    ver_arr+=("$ver")
    maj_arr+=("$maj")
  done < <(grep -E '"@tauri-apps/' "$pkg" 2>/dev/null)
done

# --- Rust side: scan every Cargo.toml in the workspace (src-tauri/,
#     cli/, etc.). Same exclusion rules as the JS scan. ---
rust_pkgs=()
while IFS= read -r p; do
  [[ -z "$p" ]] && continue
  rust_pkgs+=("$p")
done < <(find . \
  \( -path '*/node_modules' -o -path '*/.next' -o -path '*/.vite' \
     -o -path '*/.turbo' -o -path '*/.git' -o -path '*/target' \
     -o -path '*/dist' -o -path '*/unified-file-ops' \) -prune -o \
  -type f -name 'Cargo.toml' -print 2>/dev/null \
  | sed -E 's|^\./||' \
  | sort)

if [[ "${#rust_pkgs[@]}" -eq 0 ]]; then
  echo "ERROR: no Cargo.toml found in workspace" >&2
  exit 1
fi

for CARGO in "${rust_pkgs[@]}"; do
# Match lines starting with `tauri` (covers `tauri`, `tauri-build`,
# `tauri-plugin-*`). Skip if commented (line starts with `#`).
while IFS= read -r line; do
  [[ -z "$line" ]] && continue
  # Skip commented lines
  case "$line" in \#*) continue ;; esac
  # Strip line comment
  line="${line%%#*}"
  # Two shapes:
  #   tauri = "2"
  #   tauri = { version = "2", features = [...] }
  # Both extractable by finding the first quoted token after the `=`.
  name="$(echo "$line" | sed -E 's/^([A-Za-z0-9_-]+).*/\1/')"
  # Skip non-tauri entries (defensive — the grep should pre-filter)
  case "$name" in tauri|tauri-build|tauri-plugin-*) ;; *) continue ;; esac
  # Extract first quoted string after `=`
  ver="$(echo "$line" | sed -E 's/.*=[[:space:]]*"([^"]+)".*/\1/')"
  if [[ "$ver" == "$line" ]]; then
    # No quoted string — likely `version = { workspace = true }` or
    # something we can't parse; skip with empty version
    ver=""
  fi
  # If the first match was inside `{ ... }`, it might be `version = "..."`
  # nested; extract the first quoted version after that token.
  if [[ "$line" == *"{"* && "$line" == *"version"* ]]; then
    ver="$(echo "$line" | sed -E 's/.*version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/')"
  fi
  maj="$(extract_major "$ver")"
  src_arr+=("$CARGO")
  name_arr+=("$name")
  ver_arr+=("$ver")
  maj_arr+=("$maj")
done < <(grep -E '^tauri' "$CARGO" 2>/dev/null)
done  # end for CARGO loop

# ---------------------------------------------------------------------------
# Report + compare.
# ---------------------------------------------------------------------------
n="${#name_arr[@]}"
if [[ "$n" -eq 0 ]]; then
  echo "OK: no Tauri deps detected (nothing to check)"
  exit 0
fi

# Find the canonical major: the first numeric major (ignore SKIP / * / empty).
canonical=""
for m in "${maj_arr[@]}"; do
  if [[ -n "$m" && "$m" != "*" && "$m" != "SKIP" ]]; then
    canonical="$m"
    break
  fi
done

if [[ "$LIST" == true ]]; then
  echo "Canonical major: ${canonical:-<UNDETERMINED>}"
  printf '  %-35s  %-35s  %-12s  %s\n' "source" "name" "version" "major"
  printf '  %-35s  %-35s  %-12s  %s\n' "------" "----" "-------" "-----"
  for i in $(seq 0 $((n - 1))); do
    printf '  %-35s  %-35s  %-12s  %s\n' \
      "${src_arr[$i]}" "${name_arr[$i]}" "${ver_arr[$i]:-<EMPTY>}" "${maj_arr[$i]:-<EMPTY>}"
  done
fi

failed=0
fail() { echo "FAIL: $*" >&2; failed=$((failed + 1)); }

if [[ -z "$canonical" ]]; then
  fail "could not determine a canonical Tauri major version (all entries empty or wildcard)"
fi

for i in $(seq 0 $((n - 1))); do
  m="${maj_arr[$i]}"
  v="${ver_arr[$i]}"
  src="${src_arr[$i]}"
  name="${name_arr[$i]}"
  if [[ "$m" == "SKIP" ]]; then
    continue  # workspace:* / catalog:* / file: / git: — verified by other tools
  fi
  if [[ -z "$m" ]]; then
    fail "$src: $name has no parseable version (\"$v\") — cannot verify major"
    continue
  fi
  if [[ "$m" == "*" ]]; then
    continue  # wildcard accepts anything
  fi
  if [[ -n "$canonical" && "$m" != "$canonical" ]]; then
    fail "$src: $name is at major $m (\"$v\") but the project canonical major is $canonical — Tauri JS and Rust must share a major; bump them together"
  fi
done

if [[ "$failed" -eq 0 ]]; then
  echo "OK: $n Tauri dep(s), all on major version $canonical"
  exit 0
fi
exit 1
