#!/usr/bin/env bash
# check-version-sync.sh — verify the version field is identical across
# every release-relevant manifest in the workspace.
#
# Why this exists:
#   A UFOP release ships six related artifacts (desktop binary, Rust core
#   crate, CLI binary, root JS package, admin console, marketing site).
#   Each has its OWN `version` field in its OWN manifest:
#
#     package.json                "version": "X.Y.Z"   ← root JS package
#     src-tauri/tauri.conf.json   "version": "X.Y.Z"   ← what the binary
#                                                       reports + what the
#                                                       updater uses
#     src-tauri/Cargo.toml        version = "X.Y.Z"    ← Cargo crate
#     cli/Cargo.toml              version = "X.Y.Z"    ← CLI crate
#     admin/package.json          "version": "X.Y.Z"
#     marketing/package.json      "version": "X.Y.Z"
#
#   `scripts/release-local.sh --version 1.0.0` names artifacts after the
#   `--version` argument but DOES NOT auto-bump the six manifests. A
#   forgetful pre-release bump leaves you with `FileManager-1.0.0.dmg`
#   that reports its own version as `0.9.0`, an admin console claiming
#   `0.1.0` next to a desktop reporting `1.0.0`, and a Tauri updater
#   that won't recognize the bump. Same DNA as iter 5/9/10/11/12/13/14/15/16:
#   contract drift that compiles cleanly.
#
# What it checks:
#   * All six manifests above declare exactly the same version string.
#   * Pre-release/build-metadata suffixes (`1.0.0-rc.1`, `1.0.0+build.42`)
#     are honored as-is — they must still match exactly across sources.
#
#   * Cargo workspace inheritance (`version.workspace = true` in
#     `[package]`, sourced from `[workspace.package].version`) is NOT
#     supported. This codebase has no Cargo workspace so the simple
#     `version = "X.Y.Z"` form is canonical; if a workspace is ever
#     introduced, extend the toml extractor to resolve workspace inheritance.
#
# Pure-bash + grep + sed, no jq / python. Sub-second.
#
# Usage:
#   ./scripts/check-version-sync.sh         # verify; non-zero on drift
#   ./scripts/check-version-sync.sh --list  # debug: print all six values

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

LIST=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)  LIST=true; shift ;;
    -h|--help)
      sed -n '2,35p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "Unknown arg: $1 (try --help)"; exit 2 ;;
  esac
done

# ---------------------------------------------------------------------------
# Extractors. Each returns the version string or empty on failure.
# JSON: first top-level `"version": "X"` (top level is fine for these
#       files — `dependencies` entries are `"<name>": "<spec>"`, not
#       `"version": "..."`, so no false positives).
# TOML: first `^version = "X"` (the [package] table is conventionally
#       at the top of Cargo.toml; `dependencies` use either inline
#       `<name> = "X"` or `<name> = { version = "X", ... }` which is
#       NOT line-anchored to `^version`).
# ---------------------------------------------------------------------------
json_version() {
  local f="$1"
  grep -E '"version"[[:space:]]*:[[:space:]]*"[^"]+"' "$f" 2>/dev/null \
    | head -1 \
    | sed -E 's/.*"version"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/'
}

toml_version() {
  local f="$1"
  grep -E '^[[:space:]]*version[[:space:]]*=[[:space:]]*"[^"]+"' "$f" 2>/dev/null \
    | head -1 \
    | sed -E 's/^[[:space:]]*version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/'
}

# ---------------------------------------------------------------------------
# Sources of truth — six (file, extractor) pairs.
# Edit this list when adding a release-relevant manifest.
# ---------------------------------------------------------------------------
files=(
  "package.json"
  "src-tauri/tauri.conf.json"
  "src-tauri/Cargo.toml"
  "cli/Cargo.toml"
  "admin/package.json"
  "marketing/package.json"
)
kinds=(json json toml toml json json)

versions=()
missing=()

for i in "${!files[@]}"; do
  f="${files[$i]}"
  k="${kinds[$i]}"
  if [[ ! -f "$f" ]]; then
    missing+=("$f")
    versions+=("")
    continue
  fi
  if [[ "$k" == "json" ]]; then
    v="$(json_version "$f")"
  else
    v="$(toml_version "$f")"
  fi
  versions+=("$v")
done

if [[ "$LIST" == true ]]; then
  for i in "${!files[@]}"; do
    printf '  %-32s  %s\n' "${files[$i]}" "${versions[$i]:-<MISSING>}"
  done
fi

# ---------------------------------------------------------------------------
# Validate: missing files, empty versions, drift.
# ---------------------------------------------------------------------------
failed=0
fail() { echo "FAIL: $*" >&2; failed=$((failed + 1)); }

if [[ "${#missing[@]}" -gt 0 ]]; then
  for f in "${missing[@]}"; do
    fail "$f does not exist — cannot verify release version"
  done
fi

# Find the canonical version (root package.json — line 1 in our list)
canonical="${versions[0]}"
if [[ -z "$canonical" ]]; then
  fail "${files[0]} has no version field — cannot determine canonical version"
fi

for i in "${!files[@]}"; do
  v="${versions[$i]}"
  f="${files[$i]}"
  if [[ -z "$v" ]]; then
    fail "$f has no parseable version field (expected ${kinds[$i]} form)"
    continue
  fi
  if [[ -n "$canonical" && "$v" != "$canonical" ]]; then
    fail "$f reports version \"$v\" but root package.json is \"$canonical\" — partial bump? bump all six manifests in one commit before \`scripts/release-local.sh\`"
  fi
done

if [[ "$failed" -eq 0 ]]; then
  echo "OK: all 6 manifests at version $canonical"
  exit 0
fi
exit 1
