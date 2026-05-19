#!/usr/bin/env bash
# check-migrations.sh — verify migration count consistency across:
#   1. all_migrations() in src-tauri/src/storage/migrations.rs (source of truth)
#   2. test_current_version assertion in   src-tauri/src/storage/migrations.rs
#   3. test_repository_creation assertion in src-tauri/src/storage/repository.rs
#
# Why this exists:
#   CLAUDE.md flags this as a footgun: "When adding a migration, update version
#   count in test assertions (both `migrations.rs` and `repository.rs`)."
#   The Vec<Migration> grows, both assertions silently keep the old number, and
#   the failure mode is: cargo test fails ONLY when those two specific tests
#   are run, with a confusing `assert_eq!(11, 12)` instead of "you forgot to
#   bump the count". This script catches it at the verify gate instead.
#
# What it checks:
#   - Counts entries in the Vec<Migration> body of fn all_migrations()
#     (parsed as: lines matching `^\s*version: \d+,` within the function body)
#   - Pulls N from `assert_eq!(current_version(...).unwrap(), N)` in migrations.rs
#   - Pulls N from the `assert_eq!(version, N)` immediately following the
#     `SELECT MAX(version) FROM _migrations` query in repository.rs
#   - Verifies count == both assertion values
#   - Verifies version numbers in all_migrations() are 1..=N with no gaps
#     (gap = silent merge conflict resolution bug)
#
# Pure-bash, no jq / python. Runs in well under a second.
#
# Usage:
#   ./scripts/check-migrations.sh           # verify; non-zero on mismatch
#   ./scripts/check-migrations.sh --list    # debug: print parsed values

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

MIG="src-tauri/src/storage/migrations.rs"
REPO="src-tauri/src/storage/repository.rs"

LIST=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)  LIST=true; shift ;;
    -h|--help)
      sed -n '2,30p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "Unknown arg: $1 (try --help)"; exit 2 ;;
  esac
done

for f in "$MIG" "$REPO"; do
  if [[ ! -f "$f" ]]; then
    echo "ERROR: $f not found" >&2
    exit 1
  fi
done

# ---------------------------------------------------------------------------
# 1. Parse versions from fn all_migrations()
#    Range: from `^fn all_migrations` to the next `^fn ` (or EOF).
# ---------------------------------------------------------------------------
fn_start="$(grep -nE '^fn all_migrations\b' "$MIG" | head -1 | cut -d: -f1)"
if [[ -z "$fn_start" ]]; then
  echo "ERROR: could not find 'fn all_migrations' in $MIG" >&2
  exit 1
fi

# Find next top-level `fn ` after fn_start (or default to EOF).
next_fn="$(awk -v s="$fn_start" 'NR > s && /^fn / { print NR; exit }' "$MIG")"
if [[ -z "$next_fn" ]]; then
  next_fn="$(wc -l < "$MIG" | tr -d ' ')"
fi

# Collect version numbers in that range. Pattern allows both multi-line and
# inline Migration { ... version: N, ... } styles. The non-word-char prefix
# rules out `let version =`, `_version:`, `myversion:`, etc.
#
# Pre-processing strips line-comments (`// ...`, `/// ...`) and string
# literals (`"..."`) before matching, so a doc-comment or description text
# containing the substring "version: 5," cannot inflate the count. Block
# comments (`/* */`) spanning multiple lines are NOT stripped — keep doc
# notes about historical version numbers in line-comments inside the
# all_migrations() body if you must reference them.
versions=()
while IFS= read -r line; do
  [[ -n "$line" ]] && versions+=("$line")
done < <(awk -v s="$fn_start" -v e="$next_fn" 'NR > s && NR < e' "$MIG" \
  | sed -e 's|//.*||' -e 's|"[^"]*"||g' \
  | grep -oE '(^|[^a-zA-Z_])version:[[:space:]]+[0-9]+,' \
  | grep -oE '[0-9]+')

count="${#versions[@]}"
if [[ "$count" -eq 0 ]]; then
  echo "ERROR: parsed zero migrations from $MIG — extractor likely broken" >&2
  exit 1
fi

# Compute max + check for gaps (versions should be 1..count, sequential).
max=0
seen_csv=""
for v in "${versions[@]}"; do
  seen_csv="$seen_csv,$v,"
  if [[ "$v" -gt "$max" ]]; then max="$v"; fi
done

gaps=()
for ((i = 1; i <= max; i++)); do
  if [[ "$seen_csv" != *",$i,"* ]]; then
    gaps+=("$i")
  fi
done

dupes=""
for v in "${versions[@]}"; do
  cnt=0
  for u in "${versions[@]}"; do
    [[ "$u" == "$v" ]] && cnt=$((cnt + 1))
  done
  if [[ "$cnt" -gt 1 && "$dupes" != *" $v "* ]]; then
    dupes="$dupes $v "
  fi
done

# ---------------------------------------------------------------------------
# 2. Pull assertion from migrations.rs
#    There are TWO `current_version` assertions in the test (pre-migration = 0,
#    post-migration = N). Anchor on `run_migrations(&conn).unwrap();` and look
#    forward for the assertion — that's the post-migration one we want.
# ---------------------------------------------------------------------------
# There are several run_migrations() calls in the test module; the one we
# care about is followed within a few lines by a current_version assertion.
# Scan anchors in order and take the first match.
mig_assert_n=""
while IFS= read -r anchor; do
  [[ -z "$anchor" ]] && continue
  candidate="$(awk -v s="$anchor" -v e="$((anchor + 5))" \
    'NR > s && NR <= e' "$MIG" \
    | grep -oE 'assert_eq!\([[:space:]]*current_version\([^)]*\)\.unwrap\(\),[[:space:]]*[0-9]+' \
    | head -1 \
    | grep -oE '[0-9]+$')"
  if [[ -n "$candidate" ]]; then
    mig_assert_n="$candidate"
    break
  fi
done < <(grep -nE 'run_migrations\(&conn\)\.unwrap\(\);' "$MIG" | cut -d: -f1)

if [[ -z "$mig_assert_n" ]]; then
  echo "ERROR: no run_migrations() anchor in $MIG is followed by a current_version assertion within 5 lines" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# 3. Pull assertion from repository.rs
#    Anchor: line containing `MAX(version) FROM _migrations`. The matching
#    assert_eq! follows within the same #[tokio::test] body (small window).
# ---------------------------------------------------------------------------
repo_anchor="$(grep -nE 'MAX\(version\)[[:space:]]+FROM[[:space:]]+_migrations' "$REPO" \
  | head -1 | cut -d: -f1)"
if [[ -z "$repo_anchor" ]]; then
  echo "ERROR: could not find 'SELECT MAX(version) FROM _migrations' in $REPO" >&2
  exit 1
fi

# Look in the next 30 lines for `assert_eq!(version, N)`.
repo_assert_n="$(awk -v s="$repo_anchor" -v e="$((repo_anchor + 30))" \
  'NR > s && NR <= e' "$REPO" \
  | grep -oE 'assert_eq!\([[:space:]]*version[[:space:]]*,[[:space:]]*[0-9]+' \
  | head -1 \
  | grep -oE '[0-9]+$')"

if [[ -z "$repo_assert_n" ]]; then
  echo "ERROR: could not find 'assert_eq!(version, N)' after the MAX(version) query in $REPO" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Report
# ---------------------------------------------------------------------------
if [[ "$LIST" == true ]]; then
  echo "all_migrations() entries: $count (versions: ${versions[*]})"
  echo "migrations.rs assertion:  $mig_assert_n"
  echo "repository.rs assertion:  $repo_assert_n"
fi

failed=0
fail() { echo "FAIL: $*" >&2; failed=$((failed + 1)); }

if [[ "${#gaps[@]}" -gt 0 ]]; then
  fail "version gap(s) in all_migrations(): missing ${gaps[*]} (expected 1..$max sequential)"
fi
if [[ -n "$dupes" ]]; then
  fail "duplicate version(s) in all_migrations():$dupes"
fi
if [[ "$count" != "$max" ]]; then
  fail "all_migrations() has $count entries but max version is $max — non-sequential"
fi
if [[ "$mig_assert_n" != "$count" ]]; then
  fail "$MIG asserts current_version == $mig_assert_n, but all_migrations() has $count entries — bump the assertion"
fi
if [[ "$repo_assert_n" != "$count" ]]; then
  fail "$REPO asserts MAX(version) == $repo_assert_n, but all_migrations() has $count entries — bump the assertion"
fi

if [[ "$failed" -eq 0 ]]; then
  echo "OK: $count migration(s), assertions in sync (migrations.rs=$mig_assert_n, repository.rs=$repo_assert_n)"
  exit 0
fi
exit 1
