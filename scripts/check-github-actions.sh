#!/usr/bin/env bash
# check-github-actions.sh — enforce the project's "no GitHub Actions"
# policy.
#
# Why this exists:
#   This repo runs CI/CD locally and publishes container images directly
#   to GHCR from the developer's machine — by explicit policy
#   (commit 50180c3 "chore(ci): disable GitHub Actions workflows in favor
#   of local CI/CD + GHCR", and .github/workflows/README.md). The two
#   workflow files that used to live there are renamed with a `.disabled`
#   suffix so GitHub's workflow scanner ignores them.
#
#   Two real failure modes the gate previously could not catch:
#     1. A contributor (or AI) renames `ci.yml.disabled` → `ci.yml` —
#        Actions silently fires, starts billing minutes, and the local
#        CI flow + GHCR pipeline get a parallel runner stepping on the
#        same artifacts.
#     2. A new workflow file is dropped into `.github/workflows/` by
#        tooling (e.g., `gh actions create`, `nx generate workflow`).
#
#   Same DNA as iter 5/9/10/11/12/13/14/15/16/17/18/19/20 — silent
#   contract drift between policy and configuration.
#
# What it checks:
#   * `.github/workflows/` at the REPO ROOT contains zero `*.yml` /
#     `*.yaml` files. The `.disabled` suffix (`*.yml.disabled`,
#     `*.yaml.disabled`) is the only sanctioned form.
#   * `.github/workflows/README.md` exists (documents the policy so
#     re-enabling has a discoverable contrarian path).
#
# What it does NOT check:
#   * Nested `.github/workflows/` directories inside vendored trees
#     (`node_modules/`, `unified-file-ops/` snapshot, etc.). GitHub
#     Actions only fires from the ROOT directory; nested workflows are
#     inert.
#   * `.github/actions/<name>/action.yml` — GitHub Composite Actions
#     are distinct from workflows; the path `.github/actions/*` is
#     allowed.
#
# Pure-bash, no jq / python. Sub-second.
#
# Usage:
#   ./scripts/check-github-actions.sh         # verify; non-zero if a workflow is active
#   ./scripts/check-github-actions.sh --list  # debug: list found files

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

WORKFLOWS_DIR=".github/workflows"
README="$WORKFLOWS_DIR/README.md"

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

# ---------------------------------------------------------------------------
# 1. Collect every file inside the root `.github/workflows/` directory.
#    Nested paths (anything matching */.github/workflows/*) — node_modules,
#    unified-file-ops/, etc. — are intentionally excluded: GitHub Actions
#    only auto-runs workflows from the ROOT `.github/workflows/`.
# ---------------------------------------------------------------------------
if [[ ! -d "$WORKFLOWS_DIR" ]]; then
  # No workflows dir at all — vacuously satisfies the policy.
  echo "OK: $WORKFLOWS_DIR does not exist (no workflows possible)"
  exit 0
fi

active=()
inactive=()
other=()

while IFS= read -r f; do
  [[ -z "$f" ]] && continue
  base="$(basename "$f")"
  case "$base" in
    *.yml|*.yaml)         active+=("$f") ;;
    *.yml.disabled|*.yaml.disabled) inactive+=("$f") ;;
    *)                    other+=("$f") ;;
  esac
done < <(find "$WORKFLOWS_DIR" -maxdepth 1 -type f 2>/dev/null | sort)

n_active="${#active[@]}"
n_inactive="${#inactive[@]}"
n_other="${#other[@]}"

if [[ "$LIST" == true ]]; then
  echo "$WORKFLOWS_DIR contents:"
  echo "  active workflows ($n_active):"
  if [[ "$n_active" -gt 0 ]]; then
    for f in "${active[@]}"; do echo "    $f"; done
  else
    echo "    <none>"
  fi
  echo "  inactive (.disabled) ($n_inactive):"
  if [[ "$n_inactive" -gt 0 ]]; then
    for f in "${inactive[@]}"; do echo "    $f"; done
  fi
  echo "  other files ($n_other):"
  if [[ "$n_other" -gt 0 ]]; then
    for f in "${other[@]}"; do echo "    $f"; done
  fi
fi

# ---------------------------------------------------------------------------
# 2. Report.
# ---------------------------------------------------------------------------
failed=0
fail() { echo "FAIL: $*" >&2; failed=$((failed + 1)); }

if [[ "$n_active" -gt 0 ]]; then
  for f in "${active[@]}"; do
    fail "$f is an active GitHub workflow — this project disables GitHub Actions in favor of local CI/CD + GHCR. Rename to \`${f}.disabled\` (see $README for the rationale)."
  done
fi

if [[ ! -f "$README" ]]; then
  fail "$README is missing — the disabled-Actions policy must be documented so the contrarian path (re-enable) is discoverable. Restore it from git history."
fi

if [[ "$failed" -eq 0 ]]; then
  echo "OK: $n_inactive disabled workflow(s), 0 active — local CI/CD + GHCR pipeline is the only path"
  exit 0
fi
exit 1
