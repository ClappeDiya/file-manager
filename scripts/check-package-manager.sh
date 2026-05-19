#!/usr/bin/env bash
# check-package-manager.sh — fail if a non-pnpm lockfile is present in the
# workspace, or if the root `package.json` does not pin pnpm via the
# `packageManager` field.
#
# Why this exists:
#   This is a pnpm 10 workspace (root + admin + marketing + packages/*).
#   If a contributor (or AI) runs `npm install` or `yarn` by mistake, it
#   generates `package-lock.json` / `yarn.lock` alongside `pnpm-lock.yaml`.
#   pnpm still works, but the two managers compute SLIGHTLY DIFFERENT
#   dependency trees from the same `package.json` (peer-dep resolution,
#   hoisting strategy, dedupe). The build keeps passing locally and a
#   subtle "works on my machine" bug ships — exactly the silent-drift
#   class iter 5/9/10/11/12/13/14 attack from other angles.
#
#   `packageManager: "pnpm@X.Y.Z"` (Corepack convention) tells future
#   contributors and CI which manager to use. Missing or wrong → silent
#   downgrade to whatever pnpm/npm version is on PATH.
#
# What it checks:
#   1. No `package-lock.json` / `yarn.lock` / `bun.lockb` / `bun.lock`
#      anywhere in the workspace (excluding `node_modules/` and `.next/`
#      and other build/cache trees).
#   2. Root `package.json` has `"packageManager": "pnpm@<version>"`.
#
# Pure-bash, no jq / python. Sub-second.
#
# Usage:
#   ./scripts/check-package-manager.sh         # verify; non-zero on issue
#   ./scripts/check-package-manager.sh --list  # debug: show what was scanned

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

LIST=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)  LIST=true; shift ;;
    -h|--help)
      sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "Unknown arg: $1 (try --help)"; exit 2 ;;
  esac
done

if [[ ! -f "package.json" ]]; then
  echo "ERROR: no package.json at repo root ($ROOT)" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# 1. Walk the workspace for competing lockfiles. Skip vendor/cache trees
#    (`node_modules`, `.next`, `target`, `dist`, `.vite`, `.git`) because
#    transitive deps may legitimately ship their own lockfiles inside
#    node_modules/ and we only care about top-level ones the user
#    accidentally generated.
# ---------------------------------------------------------------------------
# bash 3.2 (macOS default) has no `mapfile` — use a portable read loop.
competing=()
while IFS= read -r line; do
  [[ -z "$line" ]] && continue
  competing+=("$line")
done < <(find . \
  \( -path '*/node_modules' -o -path '*/.next' -o -path '*/.git' \
     -o -path '*/target' -o -path '*/dist' -o -path '*/.vite' \
     -o -path '*/.svelte-kit' -o -path '*/.turbo' \) -prune -o \
  -type f \( -name 'package-lock.json' -o -name 'npm-shrinkwrap.json' \
             -o -name 'yarn.lock' \
             -o -name 'bun.lockb' -o -name 'bun.lock' \) -print 2>/dev/null \
  | sed -E 's|^\./||' \
  | sort)
n_competing=${#competing[@]}

# Also confirm the pnpm lockfile is present at root (sanity).
has_pnpm_lock=false
[[ -f "pnpm-lock.yaml" ]] && has_pnpm_lock=true

# ---------------------------------------------------------------------------
# 2. Parse `packageManager` from root package.json. We do this with grep +
#    sed (no jq) because `packageManager` is conventionally on its own line.
#    Accept either "pnpm@x.y.z" or "pnpm@x.y.z+sha512:…" (Corepack hashed
#    form). Reject anything else.
# ---------------------------------------------------------------------------
pm_field="$(grep -E '"packageManager"[[:space:]]*:' package.json \
  | head -1 \
  | sed -E 's/.*"packageManager"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')"

# ---------------------------------------------------------------------------
# 3. Report.
# ---------------------------------------------------------------------------
if [[ "$LIST" == true ]]; then
  echo "pnpm-lock.yaml at root: $has_pnpm_lock"
  echo "packageManager: ${pm_field:-<MISSING>}"
  if [[ "$n_competing" -gt 0 ]]; then
    echo "Competing lockfiles found:"
    for lf in "${competing[@]}"; do echo "  $lf"; done
  else
    echo "Competing lockfiles: none"
  fi
fi

failed=0
fail() { echo "FAIL: $*" >&2; failed=$((failed + 1)); }

if ! $has_pnpm_lock; then
  fail "pnpm-lock.yaml is missing at repo root — run \`pnpm install\` to generate it"
fi

if [[ "$n_competing" -gt 0 ]]; then
  for lf in "${competing[@]}"; do
    fail "non-pnpm lockfile found: $lf — delete it and run \`pnpm install\` (pnpm and npm/yarn compute different dependency trees from the same package.json)"
  done
fi

if [[ -z "$pm_field" ]]; then
  fail "root package.json has no \"packageManager\" field — add \"packageManager\": \"pnpm@<version>\" to pin Corepack"
elif [[ "$pm_field" != pnpm@* ]]; then
  fail "root package.json packageManager is \"$pm_field\" — must be \"pnpm@<version>\" (this is a pnpm-only workspace)"
fi

if [[ "$failed" -eq 0 ]]; then
  echo "OK: pnpm-lock.yaml present, packageManager=$pm_field, no competing lockfiles"
  exit 0
fi
exit 1
