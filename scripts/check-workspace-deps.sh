#!/usr/bin/env bash
# check-workspace-deps.sh — fail the gate when a shared dependency is
# installed at DIFFERENT MAJOR versions across workspace members.
# Resolved versions (from `pnpm-lock.yaml`) are authoritative; falls back
# to declared ranges in `package.json` only when the lockfile is missing.
#
# Why this exists:
#   pnpm workspaces let each member declare its own version range. When
#   two members resolve to different majors, the install graph carries
#   two copies — divergent behavior, doubled bundle size, and a real
#   security gap (a CVE may be patched in one member's resolved version
#   and standing in the other). Iter 23 surfaced this manually: admin/
#   and marketing/ both pinned `next: ^15.0.0` independently and bumping
#   one without the other was easy to forget. Same DNA as iter 5/9–25:
#   catch silent contract drift the type system does not surface.
#
# Why lockfile-resolved, not declared:
#   Iter 24 originally compared declared ranges in package.json. That
#   produced 8 noise entries on this repo: e.g. admin's `react: ^19.0.0`
#   vs root's `react: ^19.2.4` were both reported as drift even though
#   the lockfile resolves BOTH to 19.2.4 (the carets overlap on the same
#   installed version). Iter 26 fixed the signal: drift is now measured
#   on the version pnpm actually installs. 6 of the 8 originally-reported
#   minor-drift entries vanish; the one real install-time divergence
#   (lucide-react 0.460.0 vs 0.577.0 — `^0.x` is non-overlapping) is
#   preserved.
#
# What it checks:
#   * Walks the `importers:` block of `pnpm-lock.yaml` and collects every
#     (importer, dep, resolved-version) tuple from both `dependencies:`
#     and `devDependencies:`. Resolved versions like `19.2.4(react@19.2.4)`
#     are trimmed to the bare `19.2.4` before comparison.
#   * For each dep name appearing in ≥2 importers, MAJOR-version
#     mismatches FAIL. Identical-major / different-minor differences are
#     silently OK (the install graph still de-duplicates correctly).
#   * Pre-release versions are normalized to their numeric major only.
#   * `link:` / `workspace:` / `catalog:` references are skipped (these
#     are workspace-internal, not version-ranged).
#
# Fallback:
#   When `pnpm-lock.yaml` is absent or yields zero tuples, falls back to
#   parsing declared ranges in every `package.json`. Same major-drift
#   logic applies. This keeps the check usable in a fresh clone before
#   `pnpm install` has run.
#
# Allowlist:
#   A small inline list of `name:reason` pairs covers known-intentional
#   architectural splits where two majors must legitimately coexist
#   (e.g. `tailwindcss` v3 in Next.js apps until Next 16 ships v4
#   support, vs v4 in the Tauri shell). Edit ALLOWLIST below to extend.
#
# Pure-bash + grep + sed, no jq / python. Sub-second on a 5-member
# workspace.
#
# Usage:
#   ./scripts/check-workspace-deps.sh         # verify; non-zero on major drift
#   ./scripts/check-workspace-deps.sh --list  # debug: print every shared dep + the per-importer resolved version

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

LIST=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)  LIST=true; shift ;;
    -h|--help)
      sed -n '2,58p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "Unknown arg: $1 (try --help)"; exit 2 ;;
  esac
done

# ---------------------------------------------------------------------------
# Allowlist of (dep-name : reason) pairs where MAJOR drift is intentional.
# Format: `name|reason`. Empty / comment lines ignored.
# Every entry is a tech-debt marker — file a follow-up to remove on add.
# ---------------------------------------------------------------------------
ALLOWLIST=$(cat <<'EOF'
# Next.js 15 ships PostCSS-7-style Tailwind v3 expectations; the desktop
# shell is already on Tailwind v4. `tailwind-merge` follows the same
# split — v2 supports Tailwind v3, v3 supports Tailwind v4 — so the two
# entries move together. Drop both once Next 16 (with native Tailwind v4
# support) replaces 15 across admin/ and marketing/.
tailwindcss|Next.js 15 pins v3; Tauri shell on v4 — bridge until Next 16
tailwind-merge|paired with tailwindcss split — v2 ↔ Tailwind v3, v3 ↔ v4
EOF
)

is_allowlisted() {
  local name="$1"
  while IFS='|' read -r allow_name _reason; do
    case "$allow_name" in ''|\#*) continue ;; esac
    if [[ "$allow_name" == "$name" ]]; then return 0; fi
  done <<<"$ALLOWLIST"
  return 1
}

allow_reason() {
  local name="$1"
  while IFS='|' read -r allow_name reason; do
    case "$allow_name" in ''|\#*) continue ;; esac
    if [[ "$allow_name" == "$name" ]]; then echo "$reason"; return 0; fi
  done <<<"$ALLOWLIST"
}

# ---------------------------------------------------------------------------
# extract_major <version-spec> — same shape as check-tauri-versions.sh,
# kept inline so the two scripts can evolve independently. Handles both
# declared ranges (^1.2.3 / ~1.2.3 / 1.x) and lockfile-resolved bare
# versions (1.2.3 / 1.2.3-rc.1).
# ---------------------------------------------------------------------------
extract_major() {
  local v="$1"
  case "$v" in
    workspace:*|catalog:*|link:*|file:*|git+*|github:*|npm:*) echo "SKIP"; return ;;
    "*"|x|X) echo "*"; return ;;
  esac
  v="${v#^}"; v="${v#~}"; v="${v#=}"; v="${v#>}"; v="${v#<}"; v="${v#>=}"; v="${v#<=}"
  echo "$v" | sed -E 's/^([0-9]+).*/\1/'
}

# ---------------------------------------------------------------------------
# Storage. Parallel arrays (bash 3.2 has no associative arrays on the
# default macOS shell).
# ---------------------------------------------------------------------------
mem_arr=()
name_arr=()
ver_arr=()
maj_arr=()

emit_tuple() {
  local mem="$1" name="$2" ver="$3"
  local maj
  maj="$(extract_major "$ver")"
  mem_arr+=("$mem")
  name_arr+=("$name")
  ver_arr+=("$ver")
  maj_arr+=("$maj")
}

# ---------------------------------------------------------------------------
# Lockfile parser. State machine over indentation:
#   `importers:`                                  → enter importers section
#   `  <name>:`              (2 spaces)           → new importer
#   `    dependencies:` / `devDependencies:`      → enter bucket
#   `      <dep>:`           (6 spaces)           → new dep entry
#   `        version: <v>`   (8 spaces)           → resolved version
#
# Trims peer suffix from version strings: `19.2.4(react@19.2.4)` → `19.2.4`.
# `link:...` resolutions are passed through; extract_major returns SKIP.
# ---------------------------------------------------------------------------
scan_lockfile() {
  local lockfile="$1"
  local mode="outside"
  local importer="" bucket="" dep_name=""
  local emitted=0

  while IFS= read -r line; do
    # Section transitions
    if [[ "$line" == "importers:" ]]; then
      mode="importers"
      continue
    fi
    if [[ "$mode" == "importers" && "$line" =~ ^[a-z] ]]; then
      mode="outside"
      continue
    fi
    [[ "$mode" != "importers" ]] && continue

    # Importer header: exactly 2 leading spaces, name (optionally single-quoted), colon, EOL.
    # Character class explicitly excludes space so deeper-indent lines like `    dependencies:`
    # do not get eaten by the greedy `[^':]+`.
    if [[ "$line" =~ ^"  "\'?([^\':[:space:]]+)\'?:[[:space:]]*$ ]]; then
      importer="${BASH_REMATCH[1]}"
      bucket=""
      dep_name=""
      continue
    fi

    # Bucket header: exactly 4 leading spaces, dependencies|devDependencies, colon, EOL
    if [[ "$line" =~ ^"    "(dependencies|devDependencies):[[:space:]]*$ ]]; then
      bucket="${BASH_REMATCH[1]}"
      dep_name=""
      continue
    fi

    # Outside a bucket — nothing to extract
    [[ -z "$bucket" ]] && continue

    # Dep entry start: exactly 6 leading spaces, name (optionally single-quoted), colon, EOL
    if [[ "$line" =~ ^"      "\'?([@A-Za-z0-9_./-]+)\'?:[[:space:]]*$ ]]; then
      dep_name="${BASH_REMATCH[1]}"
      continue
    fi

    # Version line: exactly 8 leading spaces, "version: <something>"
    if [[ -n "$dep_name" && "$line" =~ ^"        version:"[[:space:]]+(.+)$ ]]; then
      local raw="${BASH_REMATCH[1]}"
      # Strip peer-dep suffix and any trailing whitespace
      local ver="${raw%%(*}"
      ver="${ver%% *}"
      # Normalize importer name: `.` → root package.json
      local mem
      if [[ "$importer" == "." ]]; then
        mem="package.json"
      else
        mem="$importer/package.json"
      fi
      emit_tuple "$mem" "$dep_name" "$ver"
      dep_name=""
      emitted=$((emitted + 1))
    fi
  done <"$lockfile"

  return $(( emitted == 0 ))
}

# ---------------------------------------------------------------------------
# Declared-range parser (fallback). Same shape as iter 24's original
# scan_block — kept verbatim so the fallback behaviour is unchanged.
# ---------------------------------------------------------------------------
scan_block() {
  local pkg="$1" block="$2"
  local in_block=false
  while IFS= read -r line; do
    if [[ "$in_block" == false ]]; then
      if [[ "$line" =~ \"$block\"[[:space:]]*:[[:space:]]*\{ ]]; then
        in_block=true
      fi
      continue
    fi
    if [[ "$line" =~ ^[[:space:]]{2}\}[,]?[[:space:]]*$ ]]; then
      in_block=false
      continue
    fi
    if [[ "$line" =~ \"([@A-Za-z0-9_./-]+)\"[[:space:]]*:[[:space:]]*\"([^\"]+)\" ]]; then
      emit_tuple "$pkg" "${BASH_REMATCH[1]}" "${BASH_REMATCH[2]}"
    fi
  done <"$pkg"
}

# ---------------------------------------------------------------------------
# Driver: prefer lockfile, fall back to declared scan.
# ---------------------------------------------------------------------------
LOCKFILE="pnpm-lock.yaml"
mode_note=""
if [[ -f "$LOCKFILE" ]] && scan_lockfile "$LOCKFILE"; then
  mode_note="resolved versions from $LOCKFILE"
else
  # Reset tuples in case partial lockfile scan emitted nothing usable
  mem_arr=(); name_arr=(); ver_arr=(); maj_arr=()
  pkgs=()
  while IFS= read -r p; do
    [[ -z "$p" ]] && continue
    pkgs+=("$p")
  done < <(find . \
    \( -path '*/node_modules' -o -path '*/.next' -o -path '*/.vite' \
       -o -path '*/.turbo' -o -path '*/.git' -o -path '*/target' \
       -o -path '*/dist' -o -path '*/unified-file-ops' \) -prune -o \
    -type f -name 'package.json' -print 2>/dev/null \
    | sed -E 's|^\./||' \
    | sort)

  if [[ "${#pkgs[@]}" -eq 0 ]]; then
    echo "ERROR: no package.json found in workspace" >&2
    exit 1
  fi
  for pkg in "${pkgs[@]}"; do
    scan_block "$pkg" "dependencies"
    scan_block "$pkg" "devDependencies"
  done
  mode_note="declared ranges (no pnpm-lock.yaml found)"
fi

n="${#name_arr[@]}"
if [[ "$n" -eq 0 ]]; then
  echo "OK: no declared dependencies found (nothing to check)"
  exit 0
fi

# ---------------------------------------------------------------------------
# Group by dep name. Bash 3.2 — sort+uniq the name list and rescan rather
# than use associative arrays.
# ---------------------------------------------------------------------------
unique_names="$(printf '%s\n' "${name_arr[@]}" | sort -u)"

failed=0
fail() { echo "FAIL: $*" >&2; failed=$((failed + 1)); }

minor_drift_count=0
allowed_drift_count=0
checked_shared=0

while IFS= read -r dep; do
  [[ -z "$dep" ]] && continue
  occurrences=()
  for i in $(seq 0 $((n - 1))); do
    if [[ "${name_arr[$i]}" == "$dep" ]]; then
      occurrences+=("${mem_arr[$i]}|${ver_arr[$i]}|${maj_arr[$i]}")
    fi
  done
  occ_count="${#occurrences[@]}"
  if [[ "$occ_count" -lt 2 ]]; then
    continue
  fi
  checked_shared=$((checked_shared + 1))

  canonical=""
  for o in "${occurrences[@]}"; do
    m="${o##*|}"
    if [[ -n "$m" && "$m" != "*" && "$m" != "SKIP" ]]; then
      canonical="$m"
      break
    fi
  done

  major_mismatch=false
  minor_mismatch=false
  first_ver=""
  for o in "${occurrences[@]}"; do
    ver="${o#*|}"; ver="${ver%|*}"
    m="${o##*|}"
    [[ -z "$first_ver" ]] && first_ver="$ver"
    if [[ "$m" == "SKIP" || "$m" == "*" || -z "$m" ]]; then continue; fi
    if [[ -n "$canonical" && "$m" != "$canonical" ]]; then
      major_mismatch=true
    elif [[ "$ver" != "$first_ver" ]]; then
      minor_mismatch=true
    fi
  done

  if [[ "$major_mismatch" == true ]]; then
    if is_allowlisted "$dep"; then
      allowed_drift_count=$((allowed_drift_count + 1))
      if [[ "$LIST" == true ]]; then
        echo "ALLOWED: $dep (reason: $(allow_reason "$dep"))"
        for o in "${occurrences[@]}"; do
          mem="${o%%|*}"
          ver="${o#*|}"; ver="${ver%|*}"
          echo "         $mem: $ver"
        done
      fi
    else
      fail "$dep has MAJOR-version drift across workspace members:"
      for o in "${occurrences[@]}"; do
        mem="${o%%|*}"
        ver="${o#*|}"; ver="${ver%|*}"
        m="${o##*|}"
        echo "       $mem: \"$ver\" (major=$m)" >&2
      done
      echo "       Fix: bump every member to the same major. If the drift is intentional," >&2
      echo "       add the dep to the ALLOWLIST in scripts/check-workspace-deps.sh with a" >&2
      echo "       short justification." >&2
    fi
  elif [[ "$minor_mismatch" == true ]]; then
    minor_drift_count=$((minor_drift_count + 1))
    if [[ "$LIST" == true ]]; then
      echo "INFO: $dep has minor/patch drift (same major):"
      for o in "${occurrences[@]}"; do
        mem="${o%%|*}"
        ver="${o#*|}"; ver="${ver%|*}"
        echo "      $mem: $ver"
      done
    fi
  fi
done <<<"$unique_names"

if [[ "$failed" -eq 0 ]]; then
  msg="OK: $checked_shared shared dep(s) across workspace ($mode_note); 0 major-version mismatches"
  [[ "$minor_drift_count" -gt 0 ]] && msg+="; $minor_drift_count with minor/patch drift (info)"
  [[ "$allowed_drift_count" -gt 0 ]] && msg+="; $allowed_drift_count allowlisted major split(s)"
  echo "$msg"
  exit 0
fi
exit 1
