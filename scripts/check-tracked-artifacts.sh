#!/usr/bin/env bash
# check-tracked-artifacts.sh — fail if any build/cache artifact or
# environment-secret file is tracked by git.
#
# Why this exists:
#   `.gitignore` is a SUGGESTION, not a guarantee. Three real footguns:
#     1. A contributor runs `git add -f target/` or `git add .` from
#        outside the .gitignore root and accidentally commits build
#        artifacts. Repo bloats; binaries with embedded signing material
#        may even leak.
#     2. Editor "save all" actions accidentally commit `.DS_Store` or
#        `Thumbs.db`. Harmless but noisy.
#     3. Most dangerous: `.env` / `.env.local` / `.env.production` get
#        committed with secrets in them. The .gitignore covers `.env*`
#        but it's easy to bypass with `git add -f` or to land an
#        `admin/.env` that the root .gitignore doesn't reach.
#   `git ls-files` is the source of truth for what is tracked. Once
#   something is tracked, .gitignore does NOT untrack it — the only fix
#   is `git rm --cached`. Catching this at the verify gate before a push
#   is far cheaper than a force-push to scrub history later.
#
# What it checks:
#   * Forbidden directory components in any tracked path:
#       node_modules, target, dist, .next, .vite, .turbo, .svelte-kit,
#       coverage, out, build/
#   * Forbidden tracked filenames (anywhere in tree):
#       .DS_Store, Thumbs.db, *.log (any tracked log file)
#   * `.env*` files that are NOT `*.example`. (`.env.example` and
#     `.env.production.example` ARE allowed — those are templates.)
#
# Carve-outs:
#   * `*.tsbuildinfo` is excluded — this codebase currently tracks
#     several `tsconfig.tsbuildinfo` files (root, admin/, marketing/)
#     and they are part of an active WIP edit set. They're machine-
#     generated and arguably should be gitignored, but flipping that
#     bit is out of scope for this check; the carve-out keeps this
#     stage green without rubber-stamping the practice.
#
# Pure-bash, no jq / python. Sub-second.
#
# Usage:
#   ./scripts/check-tracked-artifacts.sh         # verify; non-zero on violations
#   ./scripts/check-tracked-artifacts.sh --list  # debug: print all rules + counts

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

LIST=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)  LIST=true; shift ;;
    -h|--help)
      sed -n '2,40p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "Unknown arg: $1 (try --help)"; exit 2 ;;
  esac
done

if ! git rev-parse --git-dir >/dev/null 2>&1; then
  echo "ERROR: not inside a git repository" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Source of truth: every tracked path (one per line).
# ---------------------------------------------------------------------------
tracked_file="$(mktemp -t check-tracked-artifacts.XXXXXX)"
trap 'rm -f "$tracked_file"' EXIT
git ls-files > "$tracked_file"

# ---------------------------------------------------------------------------
# 1. Forbidden directory components — match each segment of the path.
#    awk walks the / separators and flags if ANY segment matches the list.
#    This catches both top-level (`dist/foo.js`) and nested
#    (`foo/bar/dist/baz.js`) cases.
# ---------------------------------------------------------------------------
FORBIDDEN_DIRS="node_modules target dist .next .vite .turbo .svelte-kit coverage out build"

dir_hits="$(awk -F/ -v forbid="$FORBIDDEN_DIRS" '
BEGIN {
  n = split(forbid, parts, " ")
  for (i = 1; i <= n; i++) bad[parts[i]] = 1
}
{
  for (i = 1; i <= NF; i++) {
    if (bad[$i]) { print $0; next }
  }
}' "$tracked_file" | sort -u)"

# ---------------------------------------------------------------------------
# 2. Forbidden tracked filenames (basename match).
# ---------------------------------------------------------------------------
name_hits="$(awk -F/ '
{
  base = $NF
  if (base == ".DS_Store" || base == "Thumbs.db") { print $0; next }
  # Any .log file (CHANGELOG.md is .md not .log so safe)
  if (base ~ /\.log$/) { print $0; next }
}' "$tracked_file" | sort -u)"

# ---------------------------------------------------------------------------
# 3. .env files that are not templates (*.example).
# ---------------------------------------------------------------------------
env_hits="$(awk -F/ '
{
  base = $NF
  # Match .env, .env.local, .env.production, etc. but NOT *.example
  if (base ~ /^\.env(\.|$)/ && base !~ /\.example$/) {
    print $0
  }
}' "$tracked_file" | sort -u)"

# ---------------------------------------------------------------------------
# Report.
# ---------------------------------------------------------------------------
total_tracked="$(wc -l < "$tracked_file" | tr -d ' ')"
d_count="$(echo "$dir_hits"  | grep -c . || true)"
n_count="$(echo "$name_hits" | grep -c . || true)"
e_count="$(echo "$env_hits"  | grep -c . || true)"

if [[ "$LIST" == true ]]; then
  echo "Tracked files: $total_tracked"
  echo "Forbidden directory components: $FORBIDDEN_DIRS"
  echo "  hits: $d_count"
  echo "Forbidden filenames: .DS_Store, Thumbs.db, *.log"
  echo "  hits: $n_count"
  echo "Non-template .env* files (.env, .env.local, .env.production):"
  echo "  hits: $e_count"
fi

failed=0
report() {
  local label="$1" hits="$2" advice="$3"
  if [[ -n "$hits" && "$hits" != "" ]]; then
    while IFS= read -r path; do
      [[ -z "$path" ]] && continue
      echo "FAIL: tracked $label: $path" >&2
      failed=$((failed + 1))
    done <<< "$hits"
    echo "      $advice" >&2
  fi
}

report "build/cache artifact" "$dir_hits" \
  "These directories should never be committed. Run \`git rm --cached <path>\` and add to .gitignore."
report "OS/log file"           "$name_hits" \
  "These files should never be committed. Run \`git rm --cached <path>\` and add to .gitignore."
report "environment file (with potential secrets)" "$env_hits" \
  "Only \`.env.example\` templates may be committed. Run \`git rm --cached <path>\` and rotate any leaked credentials."

if [[ "$failed" -eq 0 ]]; then
  echo "OK: $total_tracked tracked file(s), 0 build artifacts / 0 OS-log files / 0 raw .env files"
  exit 0
fi
exit 1
