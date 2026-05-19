#!/usr/bin/env bash
# check-pnpm-audit.sh — fail the gate when an unsuppressed *moderate*-or-
# worse-severity CVE is present in the workspace's production dependency
# tree. Suppressions are individual GHSA-allowlist entries with a
# justification + next-review date, identical in spirit to the iter 24/25
# allowlist convention.
#
# Why this exists:
#   The verify gate's only security-scanning stage. Pure dev-tooling
#   vulnerabilities (vitest/eslint/prettier/etc.) are out of scope by
#   convention; they never ship. The threshold catches what does ship.
#
# Threshold history:
#   * iter 22: created at `critical`. Catches only the absolute worst.
#   * iter 23: raised to `high` after the 8 standing Next.js advisories
#     (GHSA-q4gf-8mx6-v5v3 etc.) were closed by bumping Next 15.5.12 →
#     15.5.18.
#   * iter 26: raised to `moderate`. The only standing moderate is
#     GHSA-qx2v-qp2m-jg93 (PostCSS XSS via Next.js's bundled-and-pinned
#     postcss@8.4.31), which is uncorrectable from this repo's side —
#     Next 16.2.6 still ships the same pinned vulnerable version, and
#     the `admin>next>postcss` transitive path is not influenced by
#     admin's own postcss devDep. The CVE is suppressed below with an
#     explicit justification + review date. When Next.js publishes an
#     update that bumps its bundled postcss to >=8.5.10, REMOVE the
#     suppression and either (a) confirm the audit is clean, or (b)
#     surface the next backlog item.
#
# Behaviour:
#   * Exit 0 when every advisory found is allowlisted, OR when no
#     advisories at any severity are found, OR on network/registry
#     errors (offline-tolerant — local dev must not be blocked by
#     advisory-DB downtime).
#   * Exit 1 when at least one advisory is NOT allowlisted at the
#     configured threshold (`--audit-level=moderate`).
#   * Always prints the summary line so the operator sees pressure
#     even when nothing fails the gate.
#
# Suppression format:
#   Each line below is `GHSA-id|reason|review-by-YYYY-MM-DD`. Empty
#   lines and `#` comments are ignored. Add a new entry ONLY with an
#   explicit justification AND a review date. Re-evaluate every entry
#   on its review date — stale suppressions are tech debt.
#
# Pure-bash + grep + sed. Network call to npm advisory database (~1s
# with warm cache). Sub-second when offline (and tolerated).
#
# Usage:
#   ./scripts/check-pnpm-audit.sh         # verify; non-zero on unsuppressed moderate+
#   ./scripts/check-pnpm-audit.sh --list  # debug: print full audit output + suppression decisions

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# ---------------------------------------------------------------------------
# GHSA suppression list. Format: id|reason|review-YYYY-MM-DD
# Comments and empty lines are ignored.
# ---------------------------------------------------------------------------
GHSA_IGNORE=$(cat <<'EOF'
# PostCSS XSS via Unescaped </style> in CSS Stringify Output.
# Standing path: admin>next>postcss. Next.js bundles postcss as an
# exact pin (`"postcss": "8.4.31"` in next@^15 and next@^16 as of
# 2026-05-19), so the vulnerable version cannot be displaced by
# bumping admin's own postcss devDep — only an upstream Next.js
# release that bumps its bundled postcss to >=8.5.10 can close this.
# Recheck quarterly; remove this line when `pnpm audit` no longer
# reports it.
GHSA-qx2v-qp2m-jg93|PostCSS XSS via Next.js transitive — Next pins postcss@8.4.31, uncorrectable here|2026-08-19
EOF
)

is_ghsa_suppressed() {
  local target="$1"
  while IFS='|' read -r ghsa _reason _review; do
    case "$ghsa" in ''|\#*) continue ;; esac
    if [[ "$ghsa" == "$target" ]]; then return 0; fi
  done <<<"$GHSA_IGNORE"
  return 1
}

ghsa_suppression_reason() {
  local target="$1"
  while IFS='|' read -r ghsa reason review; do
    case "$ghsa" in ''|\#*) continue ;; esac
    if [[ "$ghsa" == "$target" ]]; then echo "$reason (review by $review)"; return 0; fi
  done <<<"$GHSA_IGNORE"
}

LIST=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)  LIST=true; shift ;;
    -h|--help)
      sed -n '2,53p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "Unknown arg: $1 (try --help)"; exit 2 ;;
  esac
done

if ! command -v pnpm >/dev/null 2>&1; then
  echo "ERROR: pnpm not on PATH — cannot run audit" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Run audit at the moderate threshold. Capture output + exit code.
# ---------------------------------------------------------------------------
out_file="$(mktemp -t check-pnpm-audit.XXXXXX)"
trap 'rm -f "$out_file"' EXIT

pnpm audit --audit-level=moderate --prod >"$out_file" 2>&1
audit_rc=$?

summary="$(grep -E '^Severity:|vulnerabilities found' "$out_file" 2>/dev/null \
  | tr '\n' ' ' \
  | sed -E 's/[[:space:]]+/ /g; s/^ //; s/ $//')"

# Extract every GHSA referenced in the audit table (deduped, preserves order)
found_ghsas=()
while IFS= read -r g; do
  [[ -z "$g" ]] && continue
  found_ghsas+=("$g")
done < <(grep -oE 'GHSA-[a-zA-Z0-9-]+' "$out_file" 2>/dev/null | awk '!seen[$0]++')

if [[ "$LIST" == true ]]; then
  cat "$out_file"
  echo "---"
  echo "audit exit code: $audit_rc"
  echo "summary line:    ${summary:-<none>}"
  echo "GHSAs found:     ${#found_ghsas[@]}"
  for g in "${found_ghsas[@]}"; do
    if is_ghsa_suppressed "$g"; then
      echo "  $g  SUPPRESSED  $(ghsa_suppression_reason "$g")"
    else
      echo "  $g  UNSUPPRESSED"
    fi
  done
fi

# ---------------------------------------------------------------------------
# Categorize.
#   * exit_rc 0 + non-zero findings (below threshold): pass with summary.
#   * exit_rc != 0 + no summary: network/registry error — tolerate.
#   * exit_rc != 0 + summary present: real findings — filter by suppression.
# ---------------------------------------------------------------------------
if [[ $audit_rc -eq 0 ]]; then
  if [[ -n "$summary" ]]; then
    echo "OK: 0 moderate-or-worse CVEs ($summary)"
  else
    echo "OK: 0 vulnerabilities at any severity"
  fi
  exit 0
fi

if [[ -z "$summary" ]]; then
  echo "WARN: pnpm audit failed without producing a findings summary — likely a network / registry error. Skipping security check." >&2
  echo "      Full output:" >&2
  sed 's/^/      /' "$out_file" >&2
  exit 0
fi

# Real findings — partition into suppressed vs unsuppressed.
unsuppressed=()
suppressed=()
for g in "${found_ghsas[@]}"; do
  if is_ghsa_suppressed "$g"; then
    suppressed+=("$g")
  else
    unsuppressed+=("$g")
  fi
done

if [[ "${#unsuppressed[@]}" -eq 0 ]]; then
  msg="OK: $summary; all ${#suppressed[@]} advisory(ies) explicitly suppressed"
  for g in "${suppressed[@]}"; do
    msg+=$'\n      '"$g — $(ghsa_suppression_reason "$g")"
  done
  echo "$msg"
  exit 0
fi

echo "FAIL: pnpm audit reports moderate-or-worse CVEs ($summary)" >&2
echo "      Unsuppressed: ${#unsuppressed[@]} advisory(ies):" >&2
for g in "${unsuppressed[@]}"; do
  echo "        $g" >&2
done
if [[ "${#suppressed[@]}" -gt 0 ]]; then
  echo "      Suppressed (informational): ${#suppressed[@]} advisory(ies):" >&2
  for g in "${suppressed[@]}"; do
    echo "        $g — $(ghsa_suppression_reason "$g")" >&2
  done
fi
echo "      Full output:" >&2
sed 's/^/      /' "$out_file" >&2
echo "      Fix: \`pnpm update <package>\` in the affected workspace member; if the" >&2
echo "      advisory is uncorrectable (e.g. transitive through a pinned upstream)," >&2
echo "      add a GHSA_IGNORE entry in scripts/check-pnpm-audit.sh with a justification" >&2
echo "      and review date." >&2
exit 1
