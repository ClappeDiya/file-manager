#!/usr/bin/env bash
# verify.sh — unified local gate runner for FileManager.
#
# Runs the project's lint / typecheck / test / build gates across both the
# desktop frontend (pnpm + vite + vitest) and the Rust core (cargo). This is
# the project's authoritative pre-commit / pre-release check, replacing
# scattered "run these six commands" instructions in CLAUDE.md / CONTRIBUTING.md.
#
# Why this script exists: GitHub Actions are intentionally disabled
# (see ai-memory: feedback-no-github-actions). Local gates are the only
# safety net before `release-local.sh` builds signed installers.
#
# Usage:
#   ./scripts/verify.sh              # full gate (lint + typecheck + tests + cargo + build)
#   ./scripts/verify.sh --fast       # skip slow gates (cargo test, pnpm build)
#   ./scripts/verify.sh --ci         # non-TTY output (no colors, no emoji), suitable for CI
#   ./scripts/verify.sh --only lint  # run a single stage by name
#
# Stages (run in order; each stage continues on failure but the final exit
# code is non-zero if any failed, so you see ALL problems in one pass):
#   1. lint          pnpm lint                       — ESLint over src/
#   2. typecheck     pnpm exec tsc --noEmit          — TypeScript checker
#   3. ipc-verify    scripts/dump-ipc-commands.sh    — Tauri IPC contract (both
#                                                       directions: Rust handler
#                                                       ↔ frontend tauriInvoke)
#   4. appstate      scripts/check-appstate.sh       — AppState fields ↔
#                                                       .manage() wiring
#   5. tauri-config  scripts/check-tauri-config.sh   — release-readiness sanity
#                                                       on tauri.conf.json,
#                                                       capabilities, env
#                                                       template, updater
#   6. migrations    scripts/check-migrations.sh     — all_migrations() count ↔
#                                                       test assertions in
#                                                       migrations.rs +
#                                                       repository.rs
#   7. stores        scripts/check-store-keys.sh     — Zustand persist() keys
#                                                       are unique + ufop-*
#                                                       namespaced
#   8. connectors    scripts/check-connectors.sh     — every `impl Connector`
#                                                       is wired into
#                                                       ConnectorRegistry::new()
#   9. unwraps       scripts/check-command-unwraps.sh — no `.unwrap()` /
#                                                       `.expect(` inside any
#                                                       `#[tauri::command]`
#                                                       body (panics there
#                                                       crash the IPC handler)
#  10. toolbar       scripts/check-toolbar-items.sh  — ALL_TOOLBAR_ITEMS ↔
#                                                       toolbarItems.includes
#                                                       in lockstep (no dead
#                                                       customizer entries,
#                                                       no orphan toggles)
#  11. plugins       scripts/check-tauri-plugins.sh  — every tauri-plugin-* in
#                                                       Cargo.toml is wired
#                                                       via .plugin() in lib.rs
#  12. pm            scripts/check-package-manager.sh — pnpm is the only
#                                                       lockfile; root pins
#                                                       packageManager
#  13. artifacts     scripts/check-tracked-artifacts.sh — no tracked
#                                                       build/cache dirs, no
#                                                       OS/log files, no raw
#                                                       .env (only *.example
#                                                       templates allowed)
#  14. version       scripts/check-version-sync.sh   — all 6 release
#                                                       manifests (root +
#                                                       tauri.conf + 2 Cargo
#                                                       + admin + marketing)
#                                                       share the same version
#  15. tauri-vers    scripts/check-tauri-versions.sh — every @tauri-apps/* JS
#                                                       dep + every tauri /
#                                                       tauri-plugin-* Rust
#                                                       crate share the same
#                                                       major version
#  16. hygiene       scripts/check-bash-hygiene.sh   — every scripts/*.sh has
#                                                       the portable shebang,
#                                                       a `set -uo pipefail`
#                                                       (or `-euo pipefail`),
#                                                       and the executable bit
#  17. changelog     scripts/check-changelog.sh      — CHANGELOG.md has a
#                                                       `## [X.Y.Z]` section
#                                                       matching package.json
#                                                       version
#  18. gha           scripts/check-github-actions.sh — `.github/workflows/`
#                                                       at repo root has zero
#                                                       active `*.yml` /
#                                                       `*.yaml` (only
#                                                       `*.disabled` allowed
#                                                       per local-CI policy)
#  19. audit         scripts/check-pnpm-audit.sh     — no critical-severity
#                                                       CVE in the production
#                                                       dep tree (network
#                                                       errors tolerated)
#  20. ws-deps       scripts/check-workspace-deps.sh — no MAJOR-version drift
#                                                       on shared deps across
#                                                       workspace members
#                                                       (intentional splits
#                                                       allowlisted in-script)
#  21. csp           scripts/check-tauri-csp.sh      — WebView CSP stays
#                                                       restrictive (no
#                                                       unsafe-inline /
#                                                       unsafe-eval in
#                                                       script-src, no
#                                                       wildcards anywhere,
#                                                       default-src present)
#  22. stale-supp    scripts/check-stale-suppressions.sh
#                                                    — every entry in the
#                                                       audit GHSA_IGNORE
#                                                       (check-pnpm-audit.sh)
#                                                       and RUSTSEC_IGNORE
#                                                       (check-cargo-audit.sh)
#                                                       lists has a
#                                                       review-by date
#                                                       still in the
#                                                       future; malformed
#                                                       entries also fail
#  23. caps          scripts/check-tauri-capabilities.sh
#                                                    — every sensitive Tauri
#                                                       capability (fs:*)
#                                                       ships with a non-
#                                                       empty `allow` path
#                                                       scope (iter 30)
#  24. cargo-audit   scripts/check-cargo-audit.sh    — Rust CVE scanner,
#                                                       symmetric to pnpm
#                                                       audit; RUSTSEC_IGNORE
#                                                       suppression list
#                                                       (iter 30)
#  25. ret-types     scripts/check-command-return-types.sh
#                                                    — every
#                                                       #[tauri::command]
#                                                       returns either
#                                                       Result<T, AppError>
#                                                       or bare T (iter 30)
#  26. env-example   scripts/check-env-example-drift.sh
#                                                    — every user-facing
#                                                       env var referenced
#                                                       in code is declared
#                                                       in at least one
#                                                       .env.example
#                                                       template (iter 31)
#  27. test          pnpm test                       — Vitest (frontend, ~44 files)
#  28. cargo         cargo check --workspace         — Rust type-check
#  29. cargo-test    cargo test --lib                — Rust unit tests
#  30. build         pnpm build                      — Vite production bundle
#
# Stages 3–19 are sub-second-to-~1s source-text/registry checks. They
# catch silent runtime failures (typo'd `tauriInvoke<T>("name")` callsites,
# unmanaged AppState fields, placeholder bundle identifiers / over-broad
# capabilities, stale migration count assertions, colliding localStorage
# keys, unregistered protocol connectors, panic-prone unwraps in IPC
# handlers, drifted toolbar customizer entries, dormant Tauri plugins,
# accidental npm/yarn lockfiles, accidentally committed build artifacts
# or env secrets, partially bumped release version across manifests,
# Tauri JS and Rust on different majors, scripts/*.sh missing
# set/shebang/exec, release shipped without changelog entry, re-enabled
# GitHub Actions stepping on the local CI/CD + GHCR pipeline, critical
# CVE shipping in production deps, half-bumped shared deps where one
# workspace member ends up on a different major than the others, dev
# adding 'unsafe-inline' to script-src for one debug session and
# forgetting to revert it, a CVE suppression added 18 months ago that
# nobody ever revisited even though the upstream was patched a year ago)
# that compile and type-check cleanly today.
#
# Each stage prints its own command and timing. The script exits with the
# count of failed stages (0 = green). A summary table is printed at the end.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

FAST=false
CI_MODE=false
ONLY=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --fast) FAST=true; shift ;;
    --ci) CI_MODE=true; shift ;;
    --only) ONLY="$2"; shift 2 ;;
    -h|--help)
      sed -n '2,135p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "Unknown arg: $1 (try --help)"; exit 2 ;;
  esac
done

# ---------------------------------------------------------------------------
# Output helpers
# ---------------------------------------------------------------------------
if [[ "$CI_MODE" == true || ! -t 1 ]]; then
  C_RESET=""; C_BOLD=""; C_RED=""; C_GREEN=""; C_YELLOW=""; C_DIM=""
  MARK_OK="OK"; MARK_FAIL="FAIL"; MARK_SKIP="SKIP"
else
  C_RESET=$'\033[0m'; C_BOLD=$'\033[1m'
  C_RED=$'\033[31m'; C_GREEN=$'\033[32m'; C_YELLOW=$'\033[33m'; C_DIM=$'\033[2m'
  MARK_OK="${C_GREEN}OK${C_RESET}"
  MARK_FAIL="${C_RED}FAIL${C_RESET}"
  MARK_SKIP="${C_YELLOW}SKIP${C_RESET}"
fi

# Per-stage results stored as parallel arrays (bash 3.2 has no associative
# arrays on the default macOS shell).
STAGE_NAMES=()
STAGE_STATUS=()
STAGE_SECONDS=()
FAILED=0

run_stage() {
  local name="$1" cmd="$2"
  if [[ -n "$ONLY" && "$ONLY" != "$name" ]]; then
    STAGE_NAMES+=("$name"); STAGE_STATUS+=("skip"); STAGE_SECONDS+=("0")
    return
  fi
  if [[ "$FAST" == true && ( "$name" == "cargo-test" || "$name" == "build" ) ]]; then
    printf '%b==> %s%b %s(skipped in --fast)%s\n' "$C_BOLD" "$name" "$C_RESET" "$C_DIM" "$C_RESET"
    STAGE_NAMES+=("$name"); STAGE_STATUS+=("skip"); STAGE_SECONDS+=("0")
    return
  fi

  printf '\n%b==> %s%b %s%s%s\n' "$C_BOLD" "$name" "$C_RESET" "$C_DIM" "$cmd" "$C_RESET"
  local start=$SECONDS
  set +e
  bash -c "$cmd"
  local rc=$?
  set -e
  local dur=$((SECONDS - start))

  STAGE_NAMES+=("$name"); STAGE_SECONDS+=("$dur")
  if [[ $rc -eq 0 ]]; then
    printf '%b--- %s passed in %ss%b\n' "$C_DIM" "$name" "$dur" "$C_RESET"
    STAGE_STATUS+=("ok")
  else
    printf '%b--- %s FAILED (exit %d) after %ss%b\n' "$C_RED" "$name" "$rc" "$dur" "$C_RESET"
    STAGE_STATUS+=("fail")
    FAILED=$((FAILED + 1))
  fi
}

# ---------------------------------------------------------------------------
# Stages — order matters: fast stages first so we surface problems early.
# Each stage is wrapped via `bash -c` so a failure inside the stage does not
# abort the script (we want a full report in one pass).
# ---------------------------------------------------------------------------
run_stage "lint"         "pnpm lint"
run_stage "typecheck"    "pnpm exec tsc --noEmit"
run_stage "ipc-verify"   "bash scripts/dump-ipc-commands.sh --verify"
run_stage "appstate"     "bash scripts/check-appstate.sh"
run_stage "tauri-config" "bash scripts/check-tauri-config.sh"
run_stage "migrations"   "bash scripts/check-migrations.sh"
run_stage "stores"       "bash scripts/check-store-keys.sh"
run_stage "connectors"   "bash scripts/check-connectors.sh"
run_stage "unwraps"      "bash scripts/check-command-unwraps.sh"
run_stage "toolbar"      "bash scripts/check-toolbar-items.sh"
run_stage "plugins"      "bash scripts/check-tauri-plugins.sh"
run_stage "pm"           "bash scripts/check-package-manager.sh"
run_stage "artifacts"    "bash scripts/check-tracked-artifacts.sh"
run_stage "version"      "bash scripts/check-version-sync.sh"
run_stage "tauri-vers"   "bash scripts/check-tauri-versions.sh"
run_stage "hygiene"      "bash scripts/check-bash-hygiene.sh"
run_stage "changelog"    "bash scripts/check-changelog.sh"
run_stage "gha"          "bash scripts/check-github-actions.sh"
run_stage "audit"        "bash scripts/check-pnpm-audit.sh"
run_stage "ws-deps"      "bash scripts/check-workspace-deps.sh"
run_stage "csp"          "bash scripts/check-tauri-csp.sh"
run_stage "stale-supp"   "bash scripts/check-stale-suppressions.sh"
run_stage "caps"         "bash scripts/check-tauri-capabilities.sh"
run_stage "cargo-audit"  "bash scripts/check-cargo-audit.sh"
run_stage "ret-types"    "bash scripts/check-command-return-types.sh"
run_stage "env-example"  "bash scripts/check-env-example-drift.sh"
run_stage "test"         "pnpm test"
run_stage "cargo"        "cd src-tauri && cargo check --workspace"
run_stage "cargo-test"   "cd src-tauri && cargo test --lib"
run_stage "build"        "pnpm build"

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo
printf '%b==> Summary%b\n' "$C_BOLD" "$C_RESET"
printf '%-12s  %-6s  %s\n' "stage" "status" "seconds"
printf '%-12s  %-6s  %s\n' "------------" "------" "-------"
for i in "${!STAGE_NAMES[@]}"; do
  case "${STAGE_STATUS[$i]}" in
    ok)   mark="$MARK_OK" ;;
    fail) mark="$MARK_FAIL" ;;
    skip) mark="$MARK_SKIP" ;;
    *)    mark="?" ;;
  esac
  printf '%-12s  %b  %s\n' "${STAGE_NAMES[$i]}" "$mark" "${STAGE_SECONDS[$i]}"
done
echo

if [[ $FAILED -eq 0 ]]; then
  printf '%bAll gates passed.%b\n' "$C_GREEN" "$C_RESET"
  exit 0
else
  printf '%b%d gate(s) failed.%b\n' "$C_RED" "$FAILED" "$C_RESET"
  exit 1
fi
