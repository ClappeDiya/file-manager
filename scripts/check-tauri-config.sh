#!/usr/bin/env bash
# check-tauri-config.sh — release-readiness sanity checks on Tauri config.
#
# Catches the production-blocking issues listed in the engineering-audit
# skill's Section 5E (Environment & security configuration gates):
#
#   1. Placeholder bundle identifier in src-tauri/tauri.conf.json
#      (com.tauri.dev, com.example.*, your.app, change.me, ...).
#   2. Placeholder productName (Tauri App, MyTauriApp, your-app, ...).
#   3. Placeholder/uninitialised version (0.0.0).
#   4. Over-broad Tauri capabilities (allow-all, all:allow, dangerous,
#      withGlobalTauri) in src-tauri/capabilities/*.json.
#   5. Missing .env.example at repo root.
#   6. Updater incoherence: if `plugins.updater.active = true`, then
#      pubkey must be non-empty and at least one endpoint must be set.
#
# All checks are pure-bash text scans (no jq / python dependency). They run
# in well under a second so they ride in scripts/verify.sh as a standard
# stage, catching regressions on every dev cycle — not just at release.
#
# Usage:
#   ./scripts/check-tauri-config.sh

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CONF="src-tauri/tauri.conf.json"
CAP_DIR="src-tauri/capabilities"
ENV_TEMPLATE_NAMES=(.env.example .env.template .env.sample)

failed=0
fail() { echo "FAIL:  $*" >&2; failed=$((failed + 1)); }
note() { echo "ok:    $*"; }

# ---------------------------------------------------------------------------
# 1. tauri.conf.json must exist and be valid JSON-ish (close braces match)
# ---------------------------------------------------------------------------
if [[ ! -f "$CONF" ]]; then
  fail "$CONF not found"
  echo "Summary: 1 check failed (cannot continue without $CONF)."
  exit 1
fi

# Cheap structural sanity: equal counts of { and } (rules out half-open file).
open_braces="$(tr -cd '{' < "$CONF" | wc -c | tr -d ' ')"
close_braces="$(tr -cd '}' < "$CONF" | wc -c | tr -d ' ')"
if [[ "$open_braces" != "$close_braces" ]]; then
  fail "$CONF brace mismatch ($open_braces vs $close_braces) — likely malformed JSON"
fi

# ---------------------------------------------------------------------------
# 2. Identifier placeholder check
#    Match `"identifier": "<value>"` at the top level and inspect the value.
# ---------------------------------------------------------------------------
identifier="$(grep -E '^\s*"identifier"\s*:\s*"' "$CONF" | head -1 \
  | sed -E 's/.*"identifier"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')"
if [[ -z "$identifier" ]]; then
  fail "$CONF has no top-level identifier"
else
  case "$identifier" in
    com.tauri.dev|com.example.*|com.placeholder.*|your.app|change.me|tauri-app|com.acme.*|com.foo.*)
      fail "$CONF identifier is a placeholder: \"$identifier\""
      ;;
    *)
      note "identifier = \"$identifier\""
      ;;
  esac
fi

# ---------------------------------------------------------------------------
# 3. productName placeholder check
# ---------------------------------------------------------------------------
product_name="$(grep -E '^\s*"productName"\s*:\s*"' "$CONF" | head -1 \
  | sed -E 's/.*"productName"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')"
if [[ -z "$product_name" ]]; then
  fail "$CONF has no productName"
else
  case "$product_name" in
    "Tauri App"|"MyTauriApp"|"my-tauri-app"|"your-app"|"Your App"|"My App"|"tauri-app")
      fail "$CONF productName is a placeholder: \"$product_name\""
      ;;
    *)
      note "productName = \"$product_name\""
      ;;
  esac
fi

# ---------------------------------------------------------------------------
# 4. Version sanity (no 0.0.0)
# ---------------------------------------------------------------------------
version="$(grep -E '^\s*"version"\s*:\s*"' "$CONF" | head -1 \
  | sed -E 's/.*"version"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')"
if [[ -z "$version" ]]; then
  fail "$CONF has no version"
elif [[ "$version" == "0.0.0" ]]; then
  fail "$CONF version is the placeholder 0.0.0"
else
  note "version = \"$version\""
fi

# ---------------------------------------------------------------------------
# 5. Capability sanity — flag over-broad / dangerous patterns
#    These string patterns are unique enough that false positives are
#    extremely unlikely (no real permission uses these tokens).
# ---------------------------------------------------------------------------
if [[ -d "$CAP_DIR" ]]; then
  dangerous_hits="$(grep -rnE '"(allow-all|all:allow|dangerous|withGlobalTauri)"' "$CAP_DIR" 2>/dev/null || true)"
  if [[ -n "$dangerous_hits" ]]; then
    fail "$CAP_DIR contains over-broad permission(s):"
    printf '%s\n' "$dangerous_hits" | sed 's/^/         /' >&2
  else
    note "capabilities have no allow-all / dangerous / withGlobalTauri"
  fi
else
  note "$CAP_DIR not present (skipping capability scan)"
fi

# ---------------------------------------------------------------------------
# 6. Env template exists
# ---------------------------------------------------------------------------
env_template_found=""
for name in "${ENV_TEMPLATE_NAMES[@]}"; do
  if [[ -f "$name" ]]; then
    env_template_found="$name"
    break
  fi
done
if [[ -z "$env_template_found" ]]; then
  fail "no .env.example / .env.template / .env.sample at repo root"
else
  note "env template = $env_template_found"
fi

# ---------------------------------------------------------------------------
# 7. Updater coherence — if updater.active=true, pubkey and endpoints must
#    be set. Read the file as a single string so multi-line value parsing
#    works without a JSON parser.
# ---------------------------------------------------------------------------
conf_blob="$(cat "$CONF")"

updater_active=false
if [[ "$conf_blob" =~ \"updater\"[[:space:]]*:[[:space:]]*\{[^}]*\"active\"[[:space:]]*:[[:space:]]*true ]]; then
  updater_active=true
fi

if [[ "$updater_active" == true ]]; then
  # pubkey: must exist and be non-empty
  pubkey="$(grep -E '"pubkey"\s*:\s*"' "$CONF" | head -1 \
    | sed -E 's/.*"pubkey"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/')"
  if [[ -z "$pubkey" ]]; then
    fail "$CONF updater.active=true but pubkey is empty (auto-updates cannot verify signatures)"
  else
    note "updater pubkey set (${#pubkey} chars)"
  fi

  # endpoints: must include at least one URL anywhere in the config (we don't
  # do strict JSON path scoping in pure bash — but if there's no http(s) URL
  # at all, updater.endpoints cannot have one either).
  url_count="$(grep -cE '"https?://' "$CONF" || true)"
  if [[ "$url_count" -lt 1 ]]; then
    fail "$CONF updater.active=true but no URLs found in config (updater.endpoints must be set)"
  else
    note "$url_count URL string(s) present in config (updater.endpoints presumed populated)"
  fi
else
  note "updater inactive (skipping pubkey/endpoints checks)"
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo
if [[ "$failed" -eq 0 ]]; then
  echo "OK: Tauri config / capabilities / env template are release-ready."
  exit 0
fi
echo "$failed release-readiness check(s) failed — fix before shipping." >&2
exit 1
