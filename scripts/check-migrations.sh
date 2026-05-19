#!/usr/bin/env bash
# check-migrations.sh — verify migration count consistency AND content
# discipline across the storage layer.
#
# Count/sequence checks (original iter 11 scope):
#   1. all_migrations() in src-tauri/src/storage/migrations.rs (source of truth)
#   2. test_current_version assertion in   src-tauri/src/storage/migrations.rs
#   3. test_repository_creation assertion in src-tauri/src/storage/repository.rs
#
# Content-discipline checks (iter 32 extension):
#   * Each Migration entry must have a non-empty `description`.
#   * Each entry's `sql:` field must reference a constant named exactly
#     `V<version>_SCHEMA` (codifies the naming convention so a copy-paste
#     bug like `Migration { version: 12, sql: V11_SCHEMA, … }` is caught).
#   * Each `V<N>_SCHEMA` constant must exist in the file.
#   * Each `V<N>_SCHEMA` constant body must contain at least one DDL/DML
#     keyword (CREATE/ALTER/DROP/INSERT/UPDATE/DELETE) — empty stubs are
#     a silent no-op at runtime and cause schema drift.
#   * Each `V<N>_SCHEMA` constant must be referenced exactly twice in
#     the file: once for its definition (`const V<N>_SCHEMA: &str = r#"…"#`)
#     and once inside an `all_migrations()` entry. Three references = the
#     "I forgot to rename the copy-paste" bug; one reference = orphan
#     constant (declared but never wired).
#
# Destructive-DDL safety check (iter 33 extension):
#   * Any `DROP TABLE`, `DROP INDEX`, `DROP VIEW`, `DROP TRIGGER`,
#     `ALTER TABLE … DROP COLUMN`, or `ALTER TABLE … RENAME (COLUMN|TO)`
#     statement inside a `V<N>_SCHEMA` body must be preceded — within
#     the prior 5 non-empty lines of the same body — by a comment
#     starting with `-- SAFETY:` that justifies why this data-loss-
#     potential statement is safe and how user data is preserved.
#     Default-deny: if a destructive statement lacks justification,
#     fail loudly at the gate. This is the single most catastrophic
#     failure mode for the storage layer (silent column drop = silent
#     user-data loss), so the policy is strict.
#
# Why this exists:
#   CLAUDE.md flags the count assertion as a footgun: "When adding a
#   migration, update version count in test assertions (both `migrations.rs`
#   and `repository.rs`)." The Vec<Migration> grows, both assertions
#   silently keep the old number, and the failure mode is `cargo test`
#   failing with a confusing `assert_eq!(11, 12)` instead of "you forgot
#   to bump the count". Iter 32's content checks extend this to other
#   silent migration-drift modes: empty schema bodies that produce no
#   SQL effect, mis-aimed schema constants (typed v11 but wired to
#   V10_SCHEMA), and unused stubs.
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
#   - Verifies each Migration entry has a non-empty `description`
#   - Verifies each entry's `sql:` field references `V<version>_SCHEMA`
#   - Verifies each `V<N>_SCHEMA` constant exists, has DDL/DML body,
#     and is referenced exactly twice (def + one use)
#   - Verifies destructive DDL inside V<N>_SCHEMA bodies has an
#     adjacent `-- SAFETY:` justification comment
#
# Pure-bash for count/sequence; python3 for content (multi-line Rust
# parsing). Sub-second.
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
      sed -n '2,69p' "$0" | sed 's/^# \{0,1\}//'
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

# ---------------------------------------------------------------------------
# 4. Content discipline (iter 32 extension).
#    Walk each Migration { version, description, sql } entry and validate
#    the schema constant it points to. Done in python3 since the entries
#    span multiple lines and the schema constants are r#"…"# multi-line
#    raw strings — too hairy for awk + sed.
# ---------------------------------------------------------------------------
if ! command -v python3 >/dev/null 2>&1; then
  fail "python3 not on PATH — cannot run content discipline checks"
fi

content_findings_file="$(mktemp -t check-migrations.XXXXXX)"
# Add the temp file to the cleanup trap.
trap 'rm -f "$content_findings_file"' EXIT

python3 - "$MIG" >"$content_findings_file" <<'PY'
import re
import sys
from pathlib import Path

src = Path(sys.argv[1]).read_text()

# Parse each `Migration { ... }` entry inside fn all_migrations().
# Capture: version, description, sql_const_name.
fn_match = re.search(r'fn\s+all_migrations\s*\(\s*\)\s*->\s*Vec<Migration>\s*\{(.+?)\n\}', src, re.DOTALL)
if not fn_match:
    print("PARSE-ERROR\tcould not locate fn all_migrations() body")
    sys.exit(0)
body = fn_match.group(1)

entries = []
for m in re.finditer(
    r'Migration\s*\{\s*'
    r'version\s*:\s*(\d+)\s*,\s*'
    r'description\s*:\s*"([^"]*)"\s*,\s*'
    r'sql\s*:\s*([A-Za-z_][A-Za-z_0-9]*)\s*,\s*'
    r'\}',
    body,
):
    entries.append((int(m.group(1)), m.group(2), m.group(3)))

if not entries:
    print("PARSE-ERROR\tregex extracted zero Migration entries from all_migrations() body")
    sys.exit(0)

# Index every `const V<N>_SCHEMA: &str = r#"…"#` definition body.
const_bodies = {}
for cm in re.finditer(
    r'const\s+(V\d+_SCHEMA)\s*:\s*&str\s*=\s*r#"(.+?)"#\s*;',
    src,
    re.DOTALL,
):
    const_bodies[cm.group(1)] = cm.group(2)

# Count total file-level references to each V<N>_SCHEMA token. Iter 32
# scanned the raw source which could over-count if a future doc-comment
# ever mentioned a schema constant name. Iter 33/34 refines: strip
# line-comments (`// ...`, `/// ...`, `//! ...`) and block-comments
# (`/* ... */`) before counting, so `// see V11_SCHEMA for shape` and
# `/// derived from V11_SCHEMA` no longer inflate the count.
src_no_comments = re.sub(r'/\*.*?\*/', '', src, flags=re.DOTALL)
src_no_comments = re.sub(r'//[^\n]*', '', src_no_comments)
ref_counts = {}
for tok in re.finditer(r'\bV\d+_SCHEMA\b', src_no_comments):
    ref_counts[tok.group(0)] = ref_counts.get(tok.group(0), 0) + 1

DDL_DML = re.compile(r'\b(CREATE|ALTER|DROP|INSERT|UPDATE|DELETE|PRAGMA)\b', re.IGNORECASE)
# Destructive DDL — data loss potential. Each occurrence in a V*_SCHEMA
# body must be preceded by a `-- SAFETY:` comment within the SAME body
# (iter 33). Standard SQLite syntax patterns:
#   DROP TABLE [IF EXISTS] <name>
#   DROP INDEX [IF EXISTS] <name>
#   DROP VIEW [IF EXISTS] <name>
#   DROP TRIGGER [IF EXISTS] <name>
#   ALTER TABLE <name> DROP COLUMN <col>      (SQLite ≥ 3.35)
#   ALTER TABLE <name> RENAME COLUMN <a> TO <b>
#   ALTER TABLE <name> RENAME TO <new>
DESTRUCTIVE_PATTERNS = [
    re.compile(r'\bDROP\s+(TABLE|INDEX|VIEW|TRIGGER)\b', re.IGNORECASE),
    re.compile(r'\bALTER\s+TABLE\s+\S+\s+DROP\s+COLUMN\b', re.IGNORECASE),
    re.compile(r'\bALTER\s+TABLE\s+\S+\s+RENAME\s+(COLUMN|TO)\b', re.IGNORECASE),
]
SAFETY_COMMENT = re.compile(r'^\s*--\s*SAFETY:', re.IGNORECASE)

def find_unsafe_destructive(body_text: str):
    """Yield (line_no, statement) for each destructive line that does
    not have a `-- SAFETY:` comment in the prior 5 non-empty lines."""
    lines = body_text.splitlines()
    for idx, line in enumerate(lines):
        for pat in DESTRUCTIVE_PATTERNS:
            if pat.search(line):
                # Look back up to 5 non-empty, non-comment-only-non-SAFETY
                # lines for a `-- SAFETY:` justification. The comment must
                # immediately precede (within 5 logical lines) so it's not
                # a forgotten doc-comment from much earlier in the body.
                seen = 0
                justified = False
                for back in range(idx - 1, -1, -1):
                    prev = lines[back]
                    if SAFETY_COMMENT.search(prev):
                        justified = True
                        break
                    stripped = prev.strip()
                    if not stripped:
                        continue
                    seen += 1
                    if seen >= 5:
                        break
                if not justified:
                    yield (idx + 1, line.strip())
                break  # one pattern hit per line is enough

findings = []
for version, desc, sql_name in entries:
    expected_const = f"V{version}_SCHEMA"
    if desc.strip() == "":
        findings.append(f"version {version}: description is empty")
    if sql_name != expected_const:
        findings.append(
            f"version {version}: sql references `{sql_name}` but naming "
            f"convention is V<version>_SCHEMA (expected `{expected_const}`) "
            f"— likely a copy-paste bug"
        )
    if sql_name not in const_bodies:
        findings.append(
            f"version {version}: sql references `{sql_name}` but no "
            f"`const {sql_name}: &str = r#\"…\"#;` definition was found"
        )
    else:
        body_text = const_bodies[sql_name]
        if not DDL_DML.search(body_text):
            findings.append(
                f"{sql_name}: body has no DDL/DML keyword "
                f"(CREATE/ALTER/DROP/INSERT/UPDATE/DELETE/PRAGMA) — "
                f"empty stub will silently no-op at runtime"
            )
        # iter 33: destructive-DDL safety check.
        for line_no, stmt in find_unsafe_destructive(body_text):
            findings.append(
                f"{sql_name} line {line_no}: destructive DDL without `-- SAFETY:` "
                f"justification within the prior 5 lines — `{stmt[:80]}`. "
                f"Add `-- SAFETY: <reason + data-preservation plan>` immediately "
                f"before this line, or split into a separate, scoped migration."
            )

# Reference-count check: every V<N>_SCHEMA constant should appear exactly
# twice in the file — once for the `const` definition, once inside an
# `all_migrations()` entry's `sql:` field.
for const_name in const_bodies:
    actual = ref_counts.get(const_name, 0)
    if actual != 2:
        findings.append(
            f"{const_name}: referenced {actual} time(s) in the file, "
            f"expected exactly 2 (1 definition + 1 use). "
            + ("Orphan constant — declared but no Migration entry wires it."
               if actual < 2
               else "Re-use — a second Migration entry shares this constant (copy-paste bug)?")
        )

for f in findings:
    print(f"FINDING\t{f}")

print(f"OK\tentries={len(entries)}\tconstants={len(const_bodies)}")
PY

content_errors=()
content_summary=""
while IFS=$'\t' read -r tag rest; do
  case "$tag" in
    OK)        content_summary="$rest" ;;
    FINDING)   content_errors+=("$rest") ;;
    PARSE-ERROR) fail "content discipline parse error: $rest" ;;
  esac
done <"$content_findings_file"

if [[ "${#content_errors[@]}" -gt 0 ]]; then
  for err in "${content_errors[@]}"; do
    fail "$err"
  done
fi

if [[ "$LIST" == true ]]; then
  echo "content discipline: $content_summary"
fi

if [[ "$failed" -eq 0 ]]; then
  echo "OK: $count migration(s), assertions in sync (migrations.rs=$mig_assert_n, repository.rs=$repo_assert_n); content discipline clean ($content_summary)"
  exit 0
fi
exit 1
