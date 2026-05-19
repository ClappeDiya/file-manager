#!/usr/bin/env bash
# check-stale-suppressions.sh — fail the gate when any verify-gate
# suppression has passed its `review-by` date without being re-evaluated.
#
# Why this exists:
#   Iter 22/23/24/25/26 layered allowlist / ignore-list conventions onto
#   the verify gate: pinned CVEs that can't be cleared upstream
#   (`GHSA_IGNORE` in check-pnpm-audit.sh), dep-major splits that are
#   architecturally intentional (`ALLOWLIST` in check-workspace-deps.sh).
#   Iter 26's GHSA ignore entries carry a `review-by-YYYY-MM-DD` field
#   precisely because suppressions are tech debt — they need re-evaluation
#   on a schedule. But the verify gate had no mechanism to force that
#   re-evaluation; the dates were informational only. The iter 26 audit
#   flagged this gap as "suppression rot risk."
#
#   This stage closes that gap. Every suppression with a review date is
#   parsed; if today is strictly after the review date, the gate fails
#   with a clear "re-evaluate this entry" prompt. Same DNA as iter
#   5/9–26: catch the silent failure mode the type system can't see.
#
# What it checks:
#   * `GHSA_IGNORE` heredoc body in `scripts/check-pnpm-audit.sh`.
#     Each non-comment, non-empty line must match the canonical format
#     `GHSA-id|justification|YYYY-MM-DD`. Malformed lines also FAIL
#     (the iter 26 format is the only supported shape).
#   * For every parsed entry, today vs. review-by date is compared
#     lexicographically (ISO 8601 dates sort correctly as strings —
#     no `date -d` / `date -j` BSD-vs-GNU portability gymnastics).
#
# Behaviour:
#   * Exit 0 when every suppression's review date is today or in the
#     future. Prints `OK` with a per-entry one-liner showing the days
#     remaining (or "today") for visibility.
#   * Exit 1 when any review date is strictly in the past. Prints the
#     stale entries first, then any current entries for context.
#
# Pure-bash, no jq / python / awk-extensions. Sub-second.
#
# Usage:
#   ./scripts/check-stale-suppressions.sh        # verify; non-zero on stale
#   ./scripts/check-stale-suppressions.sh --list # debug: print every parsed entry

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

AUDIT_SCRIPT="scripts/check-pnpm-audit.sh"

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

if [[ ! -f "$AUDIT_SCRIPT" ]]; then
  echo "ERROR: $AUDIT_SCRIPT not found — cannot parse GHSA_IGNORE" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Extract the `GHSA_IGNORE=$(cat <<'EOF' … EOF)` heredoc body from the
# audit script. Pure-bash state machine: enter the block on the EOF
# delimiter line, leave it on the next `EOF` at column 0.
# ---------------------------------------------------------------------------
entries=()
in_block=false
while IFS= read -r line; do
  if [[ "$in_block" == false ]]; then
    # The opening line is `GHSA_IGNORE=$(cat <<'EOF'` (or similar). We
    # match conservatively on the literal heredoc tag the audit script
    # uses to avoid coupling to any whitespace variant.
    if [[ "$line" == *"GHSA_IGNORE="*"<<'EOF'"* ]]; then
      in_block=true
    fi
    continue
  fi
  # Closing delimiter: a line that is exactly `EOF` (the heredoc
  # convention used in scripts/check-pnpm-audit.sh).
  if [[ "$line" == "EOF" ]]; then
    in_block=false
    continue
  fi
  # Skip empty + comment lines inside the block.
  case "$line" in ''|\#*) continue ;; esac
  entries+=("$line")
done <"$AUDIT_SCRIPT"

if [[ "${#entries[@]}" -eq 0 ]]; then
  # Adversarial safety: if the audit script obviously contains GHSA
  # identifiers but our parser found zero entries, the heredoc tag has
  # almost certainly drifted (e.g. someone renamed `<<'EOF'` → `<<'GHSA'`).
  # Fail loudly rather than silently report a clean state.
  if grep -qE '^GHSA-[a-zA-Z0-9-]+\|' "$AUDIT_SCRIPT"; then
    echo "FAIL: $AUDIT_SCRIPT contains GHSA-… lines but the GHSA_IGNORE heredoc block could not be parsed." >&2
    echo "      This script keys off the literal heredoc opener \`GHSA_IGNORE=\$(cat <<'EOF'\` and the closing" >&2
    echo "      \`EOF\` at column 0. If those changed, update the parser in scripts/check-stale-suppressions.sh." >&2
    exit 1
  fi
  echo "OK: 0 GHSA suppressions in $AUDIT_SCRIPT"
  exit 0
fi

# ---------------------------------------------------------------------------
# Parse + validate each entry.
# ---------------------------------------------------------------------------
today="$(date +%Y-%m-%d)"

stale=()
current=()
malformed=()

for entry in "${entries[@]}"; do
  # Required format: `GHSA-id|justification|YYYY-MM-DD`. Split on `|`.
  IFS='|' read -r ghsa reason review <<<"$entry"
  if [[ -z "${ghsa:-}" || -z "${reason:-}" || -z "${review:-}" ]]; then
    malformed+=("MISSING-FIELDS: $entry")
    continue
  fi
  if [[ ! "$ghsa" =~ ^GHSA-[a-zA-Z0-9-]+$ ]]; then
    malformed+=("BAD-ID: $entry")
    continue
  fi
  if [[ ! "$review" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]]; then
    malformed+=("BAD-DATE: $entry (expected YYYY-MM-DD, got '$review')")
    continue
  fi
  if [[ "$review" < "$today" ]]; then
    stale+=("$ghsa|$reason|$review")
  else
    current+=("$ghsa|$reason|$review")
  fi
done

# ---------------------------------------------------------------------------
# Report.
# ---------------------------------------------------------------------------
if [[ "$LIST" == true ]]; then
  echo "Today: $today"
  echo "Parsed ${#entries[@]} suppression entry(ies):"
  if [[ "${#current[@]}" -gt 0 ]]; then
    for c in "${current[@]}"; do
      IFS='|' read -r ghsa reason review <<<"$c"
      echo "  CURRENT  $ghsa  review-by $review  ($reason)"
    done
  fi
  if [[ "${#stale[@]}" -gt 0 ]]; then
    for s in "${stale[@]}"; do
      IFS='|' read -r ghsa reason review <<<"$s"
      echo "  STALE    $ghsa  review-by $review  ($reason)"
    done
  fi
  if [[ "${#malformed[@]}" -gt 0 ]]; then
    for m in "${malformed[@]}"; do
      echo "  $m"
    done
  fi
fi

failed=0

if [[ "${#malformed[@]}" -gt 0 ]]; then
  echo "FAIL: ${#malformed[@]} malformed suppression line(s) in $AUDIT_SCRIPT:" >&2
  for m in "${malformed[@]}"; do
    echo "      $m" >&2
  done
  echo "      Expected format: GHSA-<id>|<justification>|YYYY-MM-DD" >&2
  failed=1
fi

if [[ "${#stale[@]}" -gt 0 ]]; then
  echo "FAIL: ${#stale[@]} suppression(s) past review-by date (today: $today):" >&2
  for s in "${stale[@]}"; do
    IFS='|' read -r ghsa reason review <<<"$s"
    echo "      $ghsa  review-by $review  ($reason)" >&2
  done
  echo "      Fix: re-evaluate each entry. Either (a) remove it if the upstream advisory" >&2
  echo "      is now patched, or (b) extend the review-by date in $AUDIT_SCRIPT with a" >&2
  echo "      short note on what you re-confirmed." >&2
  failed=1
fi

if [[ "$failed" -ne 0 ]]; then
  exit 1
fi

echo "OK: ${#entries[@]} suppression(s), all within review window (today: $today)"
exit 0
