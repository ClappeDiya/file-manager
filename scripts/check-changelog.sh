#!/usr/bin/env bash
# check-changelog.sh — verify CHANGELOG.md documents the current release
# version (as declared in package.json).
#
# Why this exists:
#   `scripts/release-local.sh --version X.Y.Z` bumps artifact names, the
#   Tauri updater feeds, and the GHCR/Dokploy proxies — but it does not
#   touch CHANGELOG.md. Iter 17's `version` check ensures all six
#   manifests share the same version string; iter 18's `tauri-vers` check
#   keeps the Tauri ecosystem aligned. This check closes the related
#   release-time gap: shipping `FileManager-0.2.0.dmg` with no
#   `## [0.2.0]` entry in CHANGELOG.md means users see a binary version
#   bump with zero release notes.
#
#   Same DNA as iter 5/9/10/11/12/13/14/15/16/17/18/19 — silent drift
#   between two contractually-linked sources that compile cleanly today.
#
# What it checks:
#   1. `CHANGELOG.md` exists at the repo root.
#   2. The file contains at least one `## [X.Y.Z]` section heading whose
#      version exactly matches the `version` field in root `package.json`.
#   3. The Keep-a-Changelog `## [Unreleased]` section, if present, is
#      allowed — it indicates work-in-progress between releases. It does
#      NOT substitute for the versioned entry; a release is required to
#      have its own `## [X.Y.Z]` heading.
#   4. If `## [Unreleased]` IS present, its content shape is validated:
#      at least one recognized subsection header is required, every
#      subsection must contain at least one bullet entry, and subsection
#      names must come from the Keep-a-Changelog vocabulary plus this
#      project's `Tooling` extension. Added in iter 28 to close the
#      iter 27 audit finding that an empty `## [Unreleased]` heading
#      slipped past the gate.
#
# Pure-bash + grep + sed, no jq / python. Sub-second.
#
# Usage:
#   ./scripts/check-changelog.sh         # verify; non-zero if entry missing
#   ./scripts/check-changelog.sh --list  # debug: print parsed sections + version

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CHANGELOG="CHANGELOG.md"
PKG="package.json"

LIST=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)  LIST=true; shift ;;
    -h|--help)
      sed -n '2,33p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "Unknown arg: $1 (try --help)"; exit 2 ;;
  esac
done

# ---------------------------------------------------------------------------
# 1. Extract the canonical version from root package.json.
#    The iter-17 `version` check already ensures all six manifests agree;
#    here we trust root as the source of truth.
# ---------------------------------------------------------------------------
if [[ ! -f "$PKG" ]]; then
  echo "ERROR: $PKG not found" >&2
  exit 1
fi

pkg_version="$(grep -E '"version"[[:space:]]*:[[:space:]]*"[^"]+"' "$PKG" \
  | head -1 \
  | sed -E 's/.*"version"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')"

if [[ -z "$pkg_version" ]]; then
  echo "ERROR: could not parse version field from $PKG" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# 2. Verify CHANGELOG.md exists and contains the matching section.
#    Section heading shape (Keep-a-Changelog convention):
#      ## [X.Y.Z] - YYYY-MM-DD
#    We accept any optional trailing content after the version bracket;
#    only the bracketed version must match exactly. Pre-release tags
#    (e.g. `1.0.0-rc.1`) and build metadata (`1.0.0+build.42`) are
#    preserved as-is — they must match in full.
# ---------------------------------------------------------------------------
if [[ ! -f "$CHANGELOG" ]]; then
  echo "FAIL: $CHANGELOG does not exist — every release should ship with a Keep-a-Changelog changelog. Create one before tagging $pkg_version." >&2
  exit 1
fi

# Collect all `## [<token>]` headings (Unreleased + versioned).
all_sections="$(grep -E '^##[[:space:]]+\[[^]]+\]' "$CHANGELOG" \
  | sed -E 's/^##[[:space:]]+\[([^]]+)\].*/\1/')"

# Find the section matching the package version exactly.
matched=false
while IFS= read -r v; do
  [[ -z "$v" ]] && continue
  if [[ "$v" == "$pkg_version" ]]; then
    matched=true
    break
  fi
done <<< "$all_sections"

if [[ "$LIST" == true ]]; then
  echo "package.json version: $pkg_version"
  echo "CHANGELOG.md sections:"
  if [[ -z "$all_sections" ]]; then
    echo "  <none>"
  else
    while IFS= read -r v; do
      [[ -z "$v" ]] && continue
      if [[ "$v" == "$pkg_version" ]]; then
        printf '  [%s]  <-- matches package.json\n' "$v"
      else
        printf '  [%s]\n' "$v"
      fi
    done <<< "$all_sections"
  fi
fi

if ! $matched; then
  echo "FAIL: $CHANGELOG has no \`## [$pkg_version]\` section — release notes for $pkg_version are missing. Add a section before shipping, e.g.:" >&2
  printf '\n  ## [%s] - %s\n\n  ### Added\n  - …\n\n' "$pkg_version" "$(date +%Y-%m-%d)" >&2
  echo "(An optional \`## [Unreleased]\` section is allowed for work in progress but does NOT substitute for the versioned entry.)" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# 3. If `## [Unreleased]` is present, validate its content shape.
#    Empty Unreleased headers ("I bumped a dep but forgot the entry") are a
#    common drift; this check fails them at the gate instead of letting
#    them sit forever.
#
# Recognized subsection vocabulary:
#   * Keep-a-Changelog standard: Added, Changed, Deprecated, Removed, Fixed, Security
#   * This project's extension:  Tooling (verify-gate stage additions)
# ---------------------------------------------------------------------------
RECOGNIZED_SUBS="Added Changed Deprecated Removed Fixed Security Tooling"

unreleased_line="$(grep -nE '^##[[:space:]]+\[Unreleased\][[:space:]]*$' "$CHANGELOG" | head -1 | cut -d: -f1)"

if [[ -n "$unreleased_line" ]]; then
  # Find the boundary of the Unreleased block (next `## [` heading or EOF).
  next_section_line="$(awk -v start="$unreleased_line" 'NR > start && /^##[[:space:]]+\[/ {print NR; exit}' "$CHANGELOG")"
  total_lines="$(wc -l < "$CHANGELOG" | tr -d ' ')"
  if [[ -z "$next_section_line" ]]; then
    next_section_line=$((total_lines + 1))
  fi
  block_start=$((unreleased_line + 1))
  block_end=$((next_section_line - 1))

  # Walk the block: count subsections, count entries per subsection.
  current_sub=""
  current_bullets=0
  subsections_found=0
  unreleased_errors=()

  finalize_subsection() {
    if [[ -n "$current_sub" && "$current_bullets" -eq 0 ]]; then
      unreleased_errors+=("subsection \`### $current_sub\` has no entries")
    fi
  }

  if [[ "$block_start" -le "$block_end" ]]; then
    while IFS= read -r block_line; do
      if [[ "$block_line" =~ ^###[[:space:]]+([A-Z][a-zA-Z]+) ]]; then
        finalize_subsection
        current_sub="${BASH_REMATCH[1]}"
        current_bullets=0
        subsections_found=$((subsections_found + 1))
        # Validate subsection name
        if [[ " $RECOGNIZED_SUBS " != *" $current_sub "* ]]; then
          unreleased_errors+=("subsection \`### $current_sub\` is not a recognized name (allowed: $RECOGNIZED_SUBS)")
        fi
      elif [[ "$block_line" =~ ^-[[:space:]] ]] && [[ -n "$current_sub" ]]; then
        current_bullets=$((current_bullets + 1))
      fi
    done < <(sed -n "${block_start},${block_end}p" "$CHANGELOG")
  fi
  finalize_subsection

  if [[ "$subsections_found" -eq 0 ]]; then
    unreleased_errors+=("section is empty (no \`### Added/Changed/Fixed/...\` subsection)")
  fi

  if [[ "${#unreleased_errors[@]}" -gt 0 ]]; then
    echo "FAIL: $CHANGELOG \`## [Unreleased]\` content errors:" >&2
    for err in "${unreleased_errors[@]}"; do
      echo "      - $err" >&2
    done
    echo "      Fix: either remove the \`## [Unreleased]\` header (if no unreleased work) or" >&2
    echo "      populate it with a subsection from { $RECOGNIZED_SUBS } containing at least one" >&2
    echo "      \`- short description\` bullet." >&2
    exit 1
  fi

  if [[ "$LIST" == true ]]; then
    echo "  [Unreleased] block: $subsections_found subsection(s), all populated"
  fi
fi

count="$(echo "$all_sections" | grep -c .)"
echo "OK: $CHANGELOG has \`## [$pkg_version]\` section ($count section(s) total)"
exit 0
