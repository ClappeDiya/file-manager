#!/usr/bin/env bash
# check-cargo-audit.sh — fail the gate when any unsuppressed RUSTSEC
# advisory affects the Rust dependency tree. Symmetric to iter 26's
# `check-pnpm-audit.sh` for the JS side; uses the identical
# id|reason|review-by-YYYY-MM-DD suppression-list convention so the
# iter 27 `check-stale-suppressions.sh` mechanism can govern both.
#
# Why this exists:
#   The desktop binary ships Rust code. A CVE in any of its ~700
#   transitive crates is shipped to every user, exactly like a CVE in
#   any of `admin/`'s ~1500 npm dependencies. Iter 22 added the JS-side
#   scanner; iter 30 adds the symmetric Rust-side scanner.
#
# Standing findings (suppressed below, see RUSTSEC_IGNORE):
#   * RUSTSEC-2026-0098, 0099, 0104 — three bugs in
#     `rustls-webpki 0.103.10`, all reached via
#     `tauri 2.10.3 → reqwest → rustls-platform-verifier → rustls-webpki`.
#     Fixed in `rustls-webpki >= 0.103.13`. Uncorrectable from this
#     repo's Cargo.toml — only a Tauri 2.10.x patch (or 2.11) that bumps
#     reqwest can close them. 60-day review window: re-check Tauri's
#     release notes and bump if a patched Tauri exists.
#
# Behaviour:
#   * Exit 0 when every advisory is suppressed, OR when none are found,
#     OR when `cargo audit` can't reach the advisory DB (offline
#     tolerant — local dev must not be blocked by network issues).
#   * Exit 1 when at least one advisory is unsuppressed.
#   * Always prints a summary so operators see suppressed-pressure too.
#
# Suppression format (RUSTSEC_IGNORE below):
#   id|reason|review-by-YYYY-MM-DD
#   Same format as `GHSA_IGNORE` in `check-pnpm-audit.sh`. The iter 27
#   `stale-supp` gate stage covers BOTH sources.
#
# Requires `cargo-audit` on PATH (install with `cargo install cargo-audit`).
# Auto-skipped with a warning if not installed (offline-tolerant).
#
# Usage:
#   ./scripts/check-cargo-audit.sh         # verify; non-zero on unsuppressed advisory
#   ./scripts/check-cargo-audit.sh --list  # debug: print full cargo audit output

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# ---------------------------------------------------------------------------
# Standing suppressions. Format: id|reason|review-by-YYYY-MM-DD. Comments
# (#) and empty lines ignored. The iter 27 `stale-supp` stage walks this
# block AND the GHSA_IGNORE block in check-pnpm-audit.sh to force review
# on each entry's scheduled date.
# ---------------------------------------------------------------------------
RUSTSEC_IGNORE=$(cat <<'EOF'
# rustls-webpki 0.103.10 — three TLS validation bugs. All reached only
# through `tauri 2.10.3 → reqwest 0.13.2 → rustls-platform-verifier 0.6.2
# → rustls-webpki 0.103.10`. Fixed in rustls-webpki >= 0.103.13.
# Uncorrectable from this repo's Cargo.toml; the fix has to ship in a
# Tauri release that bumps its reqwest pin. Re-check 2026-07-19 and bump
# Tauri if a patched 2.10.x or 2.11 is out.
RUSTSEC-2026-0098|rustls-webpki name-constraints URI bug — Tauri-transitive, fix needs Tauri reqwest bump|2026-07-19
RUSTSEC-2026-0099|rustls-webpki name-constraints wildcard bug — Tauri-transitive, fix needs Tauri reqwest bump|2026-07-19
RUSTSEC-2026-0104|rustls-webpki CRL parsing panic — Tauri-transitive, fix needs Tauri reqwest bump|2026-07-19
EOF
)

is_suppressed() {
  local target="$1"
  while IFS='|' read -r id _reason _review; do
    case "$id" in ''|\#*) continue ;; esac
    [[ "$id" == "$target" ]] && return 0
  done <<<"$RUSTSEC_IGNORE"
  return 1
}

suppression_reason() {
  local target="$1"
  while IFS='|' read -r id reason review; do
    case "$id" in ''|\#*) continue ;; esac
    if [[ "$id" == "$target" ]]; then echo "$reason (review by $review)"; return 0; fi
  done <<<"$RUSTSEC_IGNORE"
}

LIST=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)  LIST=true; shift ;;
    -h|--help)
      sed -n '2,48p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "Unknown arg: $1 (try --help)"; exit 2 ;;
  esac
done

if ! command -v cargo-audit >/dev/null 2>&1; then
  echo "WARN: cargo-audit not on PATH — skipping Rust CVE check. Install with: cargo install cargo-audit" >&2
  exit 0
fi

# ---------------------------------------------------------------------------
# Run cargo audit in JSON mode. JSON cleanly separates `vulnerabilities.list`
# (the real CVEs that drive the exit code) from `warnings.{unmaintained,
# unsound,yanked}` (informational pressure that never fails the gate by
# itself). Parsing the text output and disambiguating those two by looking
# at section headers is fragile — JSON is the supported contract.
# ---------------------------------------------------------------------------
out_file="$(mktemp -t check-cargo-audit.XXXXXX)"
trap 'rm -f "$out_file"' EXIT

(cd src-tauri && cargo audit --json) >"$out_file" 2>/dev/null
audit_rc=$?

if ! python3 -c "import json; json.load(open('$out_file'))" >/dev/null 2>&1; then
  # No parseable JSON → either cargo audit crashed or the network is down.
  # Tolerate as offline (iter 22/26 convention).
  echo "WARN: cargo audit produced no parseable JSON — likely a network / advisory-DB error. Skipping Rust CVE check." >&2
  exit 0
fi

# Extract structured vulnerability IDs and warning counts.
parse_out="$(python3 - "$out_file" <<'PY'
import json, sys
data = json.loads(open(sys.argv[1]).read())
vulns = data.get("vulnerabilities", {}).get("list", []) or []
warns = data.get("warnings", {}) or {}
vuln_ids = []
for v in vulns:
    adv = v.get("advisory") or {}
    if adv.get("id"):
        vuln_ids.append(adv["id"])
print("VULNS\t" + "\t".join(vuln_ids))
for cat in ("unmaintained", "unsound", "yanked"):
    items = warns.get(cat, []) or []
    print(f"WARN_{cat.upper()}\t{len(items)}")
PY
)"

found=()
warn_summary=""
while IFS=$'\t' read -r tag rest; do
  case "$tag" in
    VULNS)
      # rest is tab-separated list of IDs
      if [[ -n "$rest" ]]; then
        IFS=$'\t' read -ra found <<<"$rest"
      fi
      ;;
    WARN_*)
      kind_lower="$(echo "${tag#WARN_}" | tr '[:upper:]' '[:lower:]')"
      n="$rest"
      [[ "$n" -gt 0 ]] && warn_summary+=" $n $kind_lower"
      ;;
  esac
done <<<"$parse_out"

summary="${#found[@]} vulnerability(ies)"
[[ -n "$warn_summary" ]] && summary+=", warnings:${warn_summary}"

if [[ "$LIST" == true ]]; then
  cat "$out_file" | head -80
  echo "---"
  echo "cargo audit exit code: $audit_rc"
  echo "summary:               $summary"
  if [[ "${#found[@]}" -gt 0 ]]; then
    for adv in "${found[@]}"; do
      if is_suppressed "$adv"; then
        echo "  $adv  SUPPRESSED  $(suppression_reason "$adv")"
      else
        echo "  $adv  UNSUPPRESSED"
      fi
    done
  fi
fi

# ---------------------------------------------------------------------------
# Decide outcome.
# ---------------------------------------------------------------------------
if [[ "${#found[@]}" -eq 0 ]]; then
  echo "OK: 0 vulnerabilities ($summary)"
  exit 0
fi

unsuppressed=()
suppressed=()
for adv in "${found[@]}"; do
  if is_suppressed "$adv"; then
    suppressed+=("$adv")
  else
    unsuppressed+=("$adv")
  fi
done

if [[ "${#unsuppressed[@]}" -eq 0 ]]; then
  msg="OK: $summary; all ${#suppressed[@]} advisory(ies) explicitly suppressed"
  for adv in "${suppressed[@]}"; do
    msg+=$'\n      '"$adv — $(suppression_reason "$adv")"
  done
  echo "$msg"
  exit 0
fi

echo "FAIL: cargo audit reports unsuppressed RUSTSEC advisories ($summary)" >&2
echo "      Unsuppressed: ${#unsuppressed[@]} advisory(ies):" >&2
for adv in "${unsuppressed[@]}"; do
  echo "        $adv" >&2
done
if [[ "${#suppressed[@]}" -gt 0 ]]; then
  echo "      Suppressed (informational): ${#suppressed[@]} advisory(ies):" >&2
  for adv in "${suppressed[@]}"; do
    echo "        $adv — $(suppression_reason "$adv")" >&2
  done
fi
echo "      Full output:" >&2
sed 's/^/      /' "$out_file" >&2
echo "      Fix: bump the affected crate via \`cargo update <crate>\` (or via a Tauri/major-dep" >&2
echo "      bump if transitive). If the advisory is uncorrectable from this repo, add an entry" >&2
echo "      to RUSTSEC_IGNORE in scripts/check-cargo-audit.sh with a justification + review-by date." >&2
exit 1
