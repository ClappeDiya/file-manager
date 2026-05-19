#!/usr/bin/env bash
# install-git-hooks.sh — wire scripts/verify.sh --fast into git pre-push.
#
# Why this exists:
#   The project intentionally does not use GitHub Actions (see memory:
#   feedback-no-github-actions). scripts/verify.sh is the authoritative local
#   gate, but it is voluntary — easy to skip in a rush, easy to forget. This
#   installer pulls the gate into git itself, so every `git push` runs the
#   same checks that release-local.sh expects. Pure opt-in: nothing changes
#   until a contributor runs `pnpm hooks:install`.
#
# The installed hook:
#   - Runs `bash scripts/verify.sh --fast` (≈25s, see CONTRIBUTING.md gates).
#   - Skips tag-only pushes (no code changes → nothing to verify).
#   - Carries a marker comment so re-installs and uninstalls are safe.
#   - Bypassable with `git push --no-verify` (the standard escape hatch).
#
# Usage:
#   ./scripts/install-git-hooks.sh             # install (refuses to overwrite
#                                                a non-managed pre-push hook)
#   ./scripts/install-git-hooks.sh --force     # overwrite anything in the way
#   ./scripts/install-git-hooks.sh --uninstall # remove (only if we installed it)
#   ./scripts/install-git-hooks.sh --status    # show whether installed

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

MODE="install"
FORCE=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --uninstall) MODE="uninstall"; shift ;;
    --status)    MODE="status"; shift ;;
    --force)     FORCE=true; shift ;;
    -h|--help)   sed -n '2,25p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "Unknown arg: $1 (try --help)"; exit 2 ;;
  esac
done

if ! HOOK_DIR="$(git rev-parse --git-path hooks 2>/dev/null)"; then
  echo "ERROR: not inside a git repository." >&2
  exit 1
fi
HOOK="$HOOK_DIR/pre-push"
MARKER="# Installed by scripts/install-git-hooks.sh"

case "$MODE" in
  status)
    if [[ ! -f "$HOOK" ]]; then
      echo "pre-push hook: not installed"
      exit 0
    fi
    if grep -q "$MARKER" "$HOOK"; then
      echo "pre-push hook: installed by scripts/install-git-hooks.sh"
      exit 0
    fi
    echo "pre-push hook: present but NOT installed by this script (use --force to replace)"
    exit 0
    ;;

  uninstall)
    if [[ ! -f "$HOOK" ]]; then
      echo "No pre-push hook to remove."
      exit 0
    fi
    if grep -q "$MARKER" "$HOOK"; then
      rm -f "$HOOK"
      echo "Removed $HOOK"
      exit 0
    fi
    if [[ "$FORCE" == true ]]; then
      rm -f "$HOOK"
      echo "Force-removed $HOOK (was not installed by us)."
      exit 0
    fi
    echo "ERROR: $HOOK exists but was not installed by us." >&2
    echo "Refusing to delete. Use --force to override after manual inspection." >&2
    exit 1
    ;;

  install)
    if [[ -f "$HOOK" ]] && ! grep -q "$MARKER" "$HOOK" && [[ "$FORCE" != true ]]; then
      echo "ERROR: $HOOK already exists and was not installed by us." >&2
      echo "Back it up if you want to preserve it, then re-run with --force." >&2
      exit 1
    fi

    mkdir -p "$HOOK_DIR"
    cat > "$HOOK" <<HOOK_EOF
#!/usr/bin/env bash
$MARKER  (do not edit — re-run \`pnpm hooks:install\` to refresh)
#
# Runs the local verify gate (scripts/verify.sh --fast) before any push that
# carries commits. Tag-only pushes are skipped — they introduce no code
# changes for the gate to check. Bypass with \`git push --no-verify\` only
# in genuine emergencies; the gate exists because GitHub Actions is off.

ZERO=0000000000000000000000000000000000000000
has_commits=false
while read -r local_ref local_sha remote_ref remote_sha; do
  if [[ "\$local_sha" != "\$ZERO" && "\$local_ref" != refs/tags/* ]]; then
    has_commits=true
  fi
done

if [[ "\$has_commits" != true ]]; then
  echo "[pre-push] tag-only push or branch delete — skipping verify gate."
  exit 0
fi

repo_root="\$(git rev-parse --show-toplevel)" || exit 1
cd "\$repo_root" || exit 1
echo "[pre-push] running scripts/verify.sh --fast (use 'git push --no-verify' to bypass)..."
exec bash scripts/verify.sh --fast
HOOK_EOF
    chmod +x "$HOOK"
    echo "Installed $HOOK"
    echo "  - Will run 'pnpm verify:fast' on every push that carries commits."
    echo "  - Tag-only pushes are skipped."
    echo "  - Bypass with 'git push --no-verify' (use sparingly)."
    echo "  - Remove with 'pnpm hooks:uninstall'."
    exit 0
    ;;
esac
