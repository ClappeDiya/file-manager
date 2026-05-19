#!/usr/bin/env bash
# check-stale-suppressions.sh — fail the gate when any verify-gate
# suppression has passed its `review-by` date without being re-evaluated.
#
# Why this exists:
#   Iter 22/23/24/25/26/30 layered allowlist / ignore-list conventions onto
#   the verify gate: pinned CVEs that can't be cleared upstream
#   (`GHSA_IGNORE` in check-pnpm-audit.sh, `RUSTSEC_IGNORE` in
#   check-cargo-audit.sh), dep-major splits that are architecturally
#   intentional (`ALLOWLIST` in check-workspace-deps.sh).
#   Iter 26's GHSA ignore entries carry a `review-by-YYYY-MM-DD` field
#   precisely because suppressions are tech debt — they need re-evaluation
#   on a schedule. But the verify gate had no mechanism to force that
#   re-evaluation; the dates were informational only. The iter 26 audit
#   flagged this gap as "suppression rot risk."
#
#   This stage closes that gap. Every suppression with a review date is
#   parsed; if today is strictly after the review date, the gate fails
#   with a clear "re-evaluate this entry" prompt. Same DNA as iter
#   5/9–29: catch the silent failure mode the type system can't see.
#
# What it checks:
#   * `GHSA_IGNORE` heredoc body in `scripts/check-pnpm-audit.sh`.
#   * `RUSTSEC_IGNORE` heredoc body in `scripts/check-cargo-audit.sh`
#     (iter 30 extension — added with the Rust CVE scanner).
#   * Each non-comment, non-empty line must match the canonical format
#     `<id>|justification|YYYY-MM-DD`. The id pattern is checked per
#     source: GHSA-* for the pnpm-audit script, RUSTSEC-* for the
#     cargo-audit script. Malformed lines FAIL.
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

# Source-of-truth list: (path, variable-name-prefix, id-regex). Each entry
# tells the parser where to find a suppression heredoc and what its IDs
# should look like.
SOURCES=(
  "scripts/check-pnpm-audit.sh|GHSA_IGNORE|^GHSA-[a-zA-Z0-9-]+$"
  "scripts/check-cargo-audit.sh|RUSTSEC_IGNORE|^RUSTSEC-[0-9]{4}-[0-9]+$"
)

LIST=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)  LIST=true; shift ;;
    -h|--help)
      sed -n '2,45p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "Unknown arg: $1 (try --help)"; exit 2 ;;
  esac
done

# ---------------------------------------------------------------------------
# extract_block <file> <var-name> — read the file, emit each non-empty,
# non-comment line between `<var-name>=$(cat <<'EOF'` and the next bare
# `EOF`. Pure-bash state machine.
# ---------------------------------------------------------------------------
extract_block() {
  local file="$1" var_name="$2"
  local in_block=false
  while IFS= read -r line; do
    if [[ "$in_block" == false ]]; then
      if [[ "$line" == *"${var_name}="*"<<'EOF'"* ]]; then
        in_block=true
      fi
      continue
    fi
    if [[ "$line" == "EOF" ]]; then
      in_block=false
      continue
    fi
    case "$line" in ''|\#*) continue ;; esac
    echo "$line"
  done <"$file"
}

# Parse every source. Store entries as `<source>|<raw-line>|<id-regex>`
# so the validator can apply the right pattern per source.
entries=()
sources_used=()
for src_spec in "${SOURCES[@]}"; do
  IFS='|' read -r src_path src_var src_idre <<<"$src_spec"
  if [[ ! -f "$src_path" ]]; then
    echo "ERROR: $src_path not found — declared in stale-suppressions SOURCES" >&2
    exit 1
  fi
  block_lines=()
  while IFS= read -r ln; do
    [[ -z "$ln" ]] && continue
    block_lines+=("$ln")
    entries+=("$src_path|$ln|$src_idre")
  done < <(extract_block "$src_path" "$src_var")

  # Adversarial safety: if the source file mentions IDs matching its
  # pattern but the heredoc parse found zero, the heredoc tag has drifted.
  # Fail loudly rather than silently report a clean state.
  if [[ "${#block_lines[@]}" -eq 0 ]]; then
    if grep -qE "${src_idre#^}" "$src_path" 2>/dev/null; then
      echo "FAIL: $src_path contains entries matching $src_idre but the ${src_var} heredoc block could not be parsed." >&2
      echo "      This script keys off the literal heredoc opener \`${src_var}=\$(cat <<'EOF'\` and the closing \`EOF\` at column 0." >&2
      echo "      If those changed, update the parser in scripts/check-stale-suppressions.sh." >&2
      exit 1
    fi
  fi
  sources_used+=("$src_path")
done

if [[ "${#entries[@]}" -eq 0 ]]; then
  echo "OK: 0 suppression entries across ${#sources_used[@]} source(s)"
  exit 0
fi

# ---------------------------------------------------------------------------
# Parse + validate each entry. Each entry is `<src>|<raw>|<id-regex>`.
# The raw line itself is `<id>|<reason>|<review-date>`.
# ---------------------------------------------------------------------------
today="$(date +%Y-%m-%d)"

stale=()
current=()
malformed=()

for entry in "${entries[@]}"; do
  # Split into source + raw + idre using awk so we don't get confused by
  # internal `|` characters in the reason field.
  src="${entry%%|*}"
  rest="${entry#*|}"
  idre="${rest##*|}"
  raw="${rest%|*}"

  # raw format: `<id>|<justification>|YYYY-MM-DD`
  IFS='|' read -r sup_id reason review <<<"$raw"
  if [[ -z "${sup_id:-}" || -z "${reason:-}" || -z "${review:-}" ]]; then
    malformed+=("$src: MISSING-FIELDS: $raw")
    continue
  fi
  if [[ ! "$sup_id" =~ $idre ]]; then
    malformed+=("$src: BAD-ID: $raw (expected pattern $idre)")
    continue
  fi
  if [[ ! "$review" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]]; then
    malformed+=("$src: BAD-DATE: $raw (expected YYYY-MM-DD, got '$review')")
    continue
  fi
  if [[ "$review" < "$today" ]]; then
    stale+=("$src|$sup_id|$reason|$review")
  else
    current+=("$src|$sup_id|$reason|$review")
  fi
done

# ---------------------------------------------------------------------------
# Report.
# ---------------------------------------------------------------------------
if [[ "$LIST" == true ]]; then
  echo "Today: $today"
  echo "Parsed ${#entries[@]} suppression entry(ies) across ${#sources_used[@]} source(s):"
  if [[ "${#current[@]}" -gt 0 ]]; then
    for c in "${current[@]}"; do
      IFS='|' read -r src sup_id reason review <<<"$c"
      echo "  CURRENT  $src  $sup_id  review-by $review  ($reason)"
    done
  fi
  if [[ "${#stale[@]}" -gt 0 ]]; then
    for s in "${stale[@]}"; do
      IFS='|' read -r src sup_id reason review <<<"$s"
      echo "  STALE    $src  $sup_id  review-by $review  ($reason)"
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
  echo "FAIL: ${#malformed[@]} malformed suppression line(s):" >&2
  for m in "${malformed[@]}"; do
    echo "      $m" >&2
  done
  echo "      Expected format: <id>|<justification>|YYYY-MM-DD" >&2
  failed=1
fi

if [[ "${#stale[@]}" -gt 0 ]]; then
  echo "FAIL: ${#stale[@]} suppression(s) past review-by date (today: $today):" >&2
  for s in "${stale[@]}"; do
    IFS='|' read -r src sup_id reason review <<<"$s"
    echo "      $src  $sup_id  review-by $review  ($reason)" >&2
  done
  echo "      Fix: re-evaluate each entry. Either (a) remove it if the upstream advisory" >&2
  echo "      is now patched, or (b) extend the review-by date in the source file with a" >&2
  echo "      short note on what you re-confirmed." >&2
  failed=1
fi

if [[ "$failed" -ne 0 ]]; then
  exit 1
fi

echo "OK: ${#entries[@]} suppression(s) across ${#sources_used[@]} source(s), all within review window (today: $today)"
exit 0
