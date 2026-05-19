#!/usr/bin/env bash
# check-env-example-drift.sh — fail the gate when a user-facing env var
# is referenced in code but NOT declared in any `.env.example` template.
#
# Why this exists:
#   `.env.example` is the onboarding contract. A new contributor clones
#   the repo, copies `.env.example` → `.env.local`, fills in values, and
#   the app runs. When a code change references a new env var without
#   adding it to `.env.example`, that contract breaks silently: the new
#   var defaults to undefined and the feature it gates fails at runtime
#   with no signpost. Same DNA as iter 5/9–30 — silent contract drift
#   the type system cannot see.
#
# What it flags:
#   * Code references to `process.env.<NAME>`, `import.meta.env.<NAME>`,
#     `env::var("<NAME>")`, or `env!("<NAME>")` where <NAME> matches a
#     user-facing prefix (see USER_FACING_PREFIXES below) AND <NAME> is
#     not declared in ANY `.env.example` file under the repo root.
#
# What it ALLOWS:
#   * Vars with OS-builtin / build-tool prefixes (HOME, PATH, COMSPEC,
#     CARGO_PKG_*, AWS_CONFIG_FILE, KRB*, etc.) — these are provided by
#     the runtime, not by the operator's `.env.local`.
#   * Vars declared in `.env.example` that aren't (yet) referenced by
#     code — these are intentional placeholders the operator pre-fills
#     before enabling a feature (e.g. cloud-connector API keys staged in
#     advance). Reported as INFO in `--list` mode but never fail.
#
# Scope of `.env.example` lookup:
#   Every `.env.example` reachable under the repo root is considered a
#   valid declaration site (root, admin/, marketing/, etc.). Lookup is
#   global — a var declared in admin/.env.example covers references in
#   any code location. This matches the operator experience: they fill
#   in every example file they find.
#
# Pure-bash + python3 (already required by macOS / Linux). Sub-second.
#
# Usage:
#   ./scripts/check-env-example-drift.sh        # verify; non-zero on missing vars
#   ./scripts/check-env-example-drift.sh --list # debug: print every parsed var + source

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

if ! command -v python3 >/dev/null 2>&1; then
  echo "ERROR: python3 not on PATH" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Run the scan + comparison in Python — multiline regex over the whole
# tree is naturally expressed there.
# ---------------------------------------------------------------------------
python3 - "$LIST" <<'PY'
import os
import re
import sys
from pathlib import Path

list_mode = (sys.argv[1] == "true")

# Prefixes that mean "user-facing env var" — operator is expected to
# provide via .env.local based on .env.example.
USER_FACING_PREFIXES = (
    "VITE_", "NEXT_PUBLIC_", "NEXTAUTH_", "ADMIN_", "GITHUB_", "GOOGLE_",
    "DROPBOX_", "ONEDRIVE_", "B2_", "AI_", "TAURI_", "DATABASE_URL",
    "GOOGLE_DRIVE_", "TELEMETRY_",
)
# OS-builtin / build-tool vars to NEVER flag.
OS_BUILTIN = {
    "HOME", "PATH", "USER", "TMP", "TMPDIR", "COMSPEC",
    "AWS_CONFIG_FILE", "AWS_SHARED_CREDENTIALS_FILE",
    "AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY", "AWS_DEFAULT_REGION",
    "CARGO_PKG_VERSION", "CARGO_MANIFEST_DIR",
    # Kerberos credential cache (Krb5 client lib reads these from env)
    "KRB", "KRB5CCNAME", "KRB5_CONFIG",
}
EXCLUDED_DIRS = {
    "node_modules", ".next", ".vite", ".turbo", ".git", "target", "dist",
    "unified-file-ops",  # stale snapshot
}
CODE_EXTS = {".ts", ".tsx", ".js", ".jsx", ".rs"}

PATTERNS = [
    re.compile(r"process\.env\.([A-Z][A-Z0-9_]+)"),
    re.compile(r"import\.meta\.env\.([A-Z][A-Z0-9_]+)"),
    re.compile(r'env::var\("([A-Z][A-Z0-9_]+)"\)'),
    re.compile(r'env!\("([A-Z][A-Z0-9_]+)"\)'),
]

def iter_code_files(root="."):
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in EXCLUDED_DIRS]
        for f in filenames:
            if Path(f).suffix in CODE_EXTS:
                yield Path(dirpath) / f

# 1) Collect every env var referenced by code, with one example location.
code_refs = {}  # var -> first file path where it was seen
for f in iter_code_files("."):
    try:
        text = f.read_text(errors="ignore")
    except Exception:
        continue
    for pat in PATTERNS:
        for m in pat.finditer(text):
            v = m.group(1)
            code_refs.setdefault(v, str(f))

def is_user_facing(v):
    if v in OS_BUILTIN:
        return False
    return v.startswith(USER_FACING_PREFIXES)

user_facing = {v: f for v, f in code_refs.items() if is_user_facing(v)}

# 2) Collect every var declared in any .env.example reachable under root.
example_decls = {}  # var -> [files]
for ef in Path(".").rglob(".env.example"):
    parts = set(ef.parts)
    if parts & EXCLUDED_DIRS:
        continue
    for line in ef.read_text(errors="ignore").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        m = re.match(r"([A-Z][A-Z0-9_]+)\s*=", line)
        if m:
            example_decls.setdefault(m.group(1), []).append(str(ef))

missing = sorted(set(user_facing) - set(example_decls))
dead = sorted(set(example_decls) - set(code_refs))

if list_mode:
    print(f"Code refs (user-facing): {len(user_facing)}")
    for v in sorted(user_facing):
        decl = ", ".join(example_decls.get(v, ["<MISSING>"]))
        print(f"  {v}  used at {user_facing[v]}; declared in {decl}")
    print(f"Dead in .env.example (info only): {len(dead)}")
    for v in dead:
        print(f"  {v}  declared in {', '.join(example_decls[v])} but never referenced")

if missing:
    print(f"FAIL: {len(missing)} user-facing env var(s) referenced in code but missing from every .env.example:", file=sys.stderr)
    for v in missing:
        print(f"      {v}  (e.g. {user_facing[v]})", file=sys.stderr)
    print(f"      Fix: add the missing var(s) to the .env.example whose scope matches the code", file=sys.stderr)
    print(f"      (root .env.example for desktop / Vite vars, admin/.env.example for admin-only,", file=sys.stderr)
    print(f"      marketing/.env.example for marketing-only). Use empty value if no default.", file=sys.stderr)
    sys.exit(1)

dead_note = ""
if dead:
    dead_note = f"; {len(dead)} dead entry(ies) in .env.example (info — likely pre-declared placeholders)"

print(f"OK: {len(user_facing)} user-facing env var(s), all declared in at least one .env.example{dead_note}")
PY
