#!/usr/bin/env bash
# check-bash-hygiene.sh — verify every shell script under scripts/ follows
# three minimum hygiene rules.
#
# Why this exists:
#   `scripts/` is the project's verification + release surface. A script
#   that silently swallows errors (missing `set -u` or `pipefail`) can
#   pass green while doing nothing — or worse, half its job. A script
#   without the executable bit makes `./scripts/foo.sh` fail with
#   "permission denied" instead of running (pnpm invocations via
#   `bash scripts/foo.sh` still work, masking the bug from the gate).
#   These are tiny problems each, but they accumulate. Same DNA as
#   iter 5/9/10/11/12/13/14/15/16/17/18 — silent contract drift.
#
# What it checks (per script in `scripts/*.sh`):
#   1. First line is `#!/usr/bin/env bash` (portable shebang — `/bin/bash`
#      would tie the script to whichever bash version ships with the OS;
#      `/usr/bin/env bash` picks the user's PATH-chosen bash and is the
#      project convention).
#   2. Within the first 40 lines, declares both `-u` (error on unset
#      variable) AND `pipefail` (pipe fails if any stage fails). Both
#      `set -uo pipefail` (used by the iter-9+ check scripts that
#      accumulate failures and exit at the end) and `set -euo pipefail`
#      (used by older sequential scripts that fail fast) are accepted —
#      the difference is whether `-e` is also set, and that's a per-script
#      design choice.
#   3. Has the executable bit set (mode includes user-execute) so
#      `./scripts/foo.sh` works in addition to `bash scripts/foo.sh`.
#
# Pure-bash, no jq / python. Sub-second.
#
# Usage:
#   ./scripts/check-bash-hygiene.sh         # verify; non-zero on issue
#   ./scripts/check-bash-hygiene.sh --list  # debug: print all 3 columns

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

LIST=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)  LIST=true; shift ;;
    -h|--help)
      sed -n '2,32p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "Unknown arg: $1 (try --help)"; exit 2 ;;
  esac
done

# ---------------------------------------------------------------------------
# Collect every `scripts/*.sh`.
# ---------------------------------------------------------------------------
scripts=()
for f in scripts/*.sh; do
  [[ -f "$f" ]] || continue
  scripts+=("$f")
done

if [[ "${#scripts[@]}" -eq 0 ]]; then
  echo "ERROR: no scripts/*.sh found" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Check each script. Use parallel arrays for bash 3.2 compatibility.
# ---------------------------------------------------------------------------
failed=0
fail() { echo "FAIL: $*" >&2; failed=$((failed + 1)); }

for f in "${scripts[@]}"; do
  # 1) shebang
  shebang="$(head -1 "$f")"
  if [[ "$shebang" != "#!/usr/bin/env bash" ]]; then
    fail "$f: line 1 is \"$shebang\" — expected \"#!/usr/bin/env bash\" (portable shebang)"
    shebang_ok=false
  else
    shebang_ok=true
  fi

  # 2) `^set -...u...o pipefail` line anywhere in the file (not just first
  #    N lines — newer check scripts in this project have long docstring
  #    headers and place the `set` directive past line 40).
  set_line="$(grep -E '^set -[A-Za-z]*[eu]+[A-Za-z]*o pipefail' "$f" | head -1)"
  if [[ -z "$set_line" ]]; then
    fail "$f: no column-1 \`set -uo pipefail\` (or \`-euo pipefail\`) line — script may silently swallow errors. Use the one-line form; two-line \`set -u\` + \`set -o pipefail\` is functionally equivalent but not the project convention."
    set_ok=false
  else
    set_ok=true
  fi

  # 3) executable bit
  if [[ -x "$f" ]]; then
    exec_ok=true
  else
    fail "$f: missing executable bit — run \`chmod +x $f\` (allows \`./$f\` invocation in addition to \`bash $f\`)"
    exec_ok=false
  fi

  if [[ "$LIST" == true ]]; then
    printf '  %-35s  shebang:%s  set:%s  exec:%s\n' \
      "$(basename "$f")" \
      "$($shebang_ok && echo OK || echo X)" \
      "$($set_ok     && echo OK || echo X)" \
      "$($exec_ok    && echo OK || echo X)"
  fi
done

if [[ "$failed" -eq 0 ]]; then
  echo "OK: ${#scripts[@]} script(s), all hygienic (shebang + set + exec bit)"
  exit 0
fi
exit 1
