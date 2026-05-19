#!/usr/bin/env bash
# check-tauri-csp.sh — verify the Tauri WebView Content-Security-Policy
# stays restrictive. Catches the classic "dev added 'unsafe-inline' for
# debugging and forgot to remove it" regression.
#
# Why this exists:
#   The Tauri shell renders the desktop UI inside a WebView. The CSP
#   declared in `src-tauri/tauri.conf.json -> app.security.csp` is the
#   only browser-level defense against XSS via injected HTML / SVG / drag-
#   in content. Iter 10's `check-tauri-config.sh` covers structural sins
#   (placeholder identifier, allow-all capability, missing pubkey) but
#   not the CSP string's *content*. Adding `'unsafe-inline'` to
#   `script-src` or a wildcard to `connect-src` for one debug session and
#   forgetting to revert it is the textbook silent-regression: nothing
#   else fails, the WebView still loads, and the sandbox is gone. Same
#   DNA as iter 5/9–24: catch invariants the type system does not surface.
#
# What it checks:
#   * `app.security.csp` MUST be present and non-empty in tauri.conf.json.
#   * `default-src` directive MUST be declared (anchor of fallback policy).
#   * `script-src` MUST NOT contain `'unsafe-inline'` or `'unsafe-eval'`
#     (the two XSS-defense-eliminating script sources).
#   * No directive may contain a bare `*` wildcard source.
#   * No directive may contain a scheme-only source other than the
#     Tauri-approved set (`asset:`, `data:` is rejected for script-src
#     specifically, others informational).
#
# What it ALLOWS (intentional):
#   * `style-src 'unsafe-inline'` — required by Tailwind / shadcn-ui's
#     runtime style injection. Industry-accepted CSP convention.
#   * `img-src asset:` and `img-src https://asset.localhost` — Tauri-
#     internal local-file image protocol.
#   * `connect-src` to specific hosts (any HTTPS literal) — used by the
#     updater and feature-flag fetches.
#
# Pure-bash + grep + sed, no jq / python. Sub-second.
#
# Usage:
#   ./scripts/check-tauri-csp.sh         # verify; non-zero on regression
#   ./scripts/check-tauri-csp.sh --list  # debug: print every parsed directive

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CONF="src-tauri/tauri.conf.json"

LIST=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)  LIST=true; shift ;;
    -h|--help)
      sed -n '2,42p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "Unknown arg: $1 (try --help)"; exit 2 ;;
  esac
done

if [[ ! -f "$CONF" ]]; then
  echo "ERROR: $CONF not found" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Extract the CSP string. The conf is JSON, but we keep dependency-free:
# grep the line beginning with `"csp"` (possibly nested under `security`),
# then capture the quoted value. Multi-line CSP strings are not supported
# by Tauri itself, so a single-line regex is correct.
# ---------------------------------------------------------------------------
csp_line="$(grep -E '^[[:space:]]*"csp"[[:space:]]*:' "$CONF" 2>/dev/null | head -n 1)"
if [[ -z "$csp_line" ]]; then
  echo "FAIL: no \"csp\" key found in $CONF (app.security.csp must be set)" >&2
  exit 1
fi

csp_val="$(echo "$csp_line" | sed -E 's/.*"csp"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/')"
if [[ -z "$csp_val" || "$csp_val" == "$csp_line" ]]; then
  echo "FAIL: could not parse CSP value from line:" >&2
  echo "      $csp_line" >&2
  echo "      Expected: \"csp\": \"<policy>\"" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Tokenize into directives. Each directive is `<name> <src1> <src2> ...`.
# Trim whitespace and skip empties.
# ---------------------------------------------------------------------------
directives=()
IFS=';' read -r -a parts <<<"$csp_val"
for p in "${parts[@]}"; do
  # Trim leading/trailing whitespace
  p="$(echo "$p" | sed -E 's/^[[:space:]]+//; s/[[:space:]]+$//')"
  [[ -z "$p" ]] && continue
  directives+=("$p")
done

if [[ "${#directives[@]}" -eq 0 ]]; then
  echo "FAIL: CSP value is empty after parsing — \"$csp_val\"" >&2
  exit 1
fi

if [[ "$LIST" == true ]]; then
  echo "CSP value: $csp_val"
  echo "Parsed directives:"
  for d in "${directives[@]}"; do
    echo "  $d"
  done
fi

# ---------------------------------------------------------------------------
# Build a map of directive name -> source list. Bash 3.2 has no associative
# arrays — use parallel arrays.
# ---------------------------------------------------------------------------
dir_names=()
dir_srcs=()
for d in "${directives[@]}"; do
  name="${d%% *}"
  srcs="${d#* }"
  # If the directive has no sources (just the name), `srcs` will be == `d`.
  if [[ "$srcs" == "$d" ]]; then
    srcs=""
  fi
  dir_names+=("$name")
  dir_srcs+=("$srcs")
done

# Lookup helper
get_sources() {
  local target="$1"
  for i in $(seq 0 $((${#dir_names[@]} - 1))); do
    if [[ "${dir_names[$i]}" == "$target" ]]; then
      echo "${dir_srcs[$i]}"
      return 0
    fi
  done
  return 1
}

failed=0
fail() { echo "FAIL: $*" >&2; failed=$((failed + 1)); }

# ---------------------------------------------------------------------------
# Rule 1: default-src must be declared.
# ---------------------------------------------------------------------------
if ! get_sources "default-src" >/dev/null; then
  fail "default-src directive is missing — every CSP must declare an explicit fallback"
fi

# ---------------------------------------------------------------------------
# Rule 2: script-src must not contain 'unsafe-inline' or 'unsafe-eval'.
#         If script-src is absent it inherits default-src, so check that one.
# ---------------------------------------------------------------------------
script_srcs="$(get_sources "script-src" || get_sources "default-src" || echo "")"
if [[ "$script_srcs" == *"'unsafe-inline'"* ]]; then
  fail "script-src contains 'unsafe-inline' — removes XSS protection in the WebView"
fi
if [[ "$script_srcs" == *"'unsafe-eval'"* ]]; then
  fail "script-src contains 'unsafe-eval' — removes XSS protection in the WebView"
fi

# ---------------------------------------------------------------------------
# Rule 3: no bare `*` wildcard source in any directive.
#         (Scheme wildcards like `https:` are not flagged here; they are
#         informational and intentional uses exist.)
#
# `set -f` disables pathname expansion so the `*` token does not get
# glob-expanded to the current directory's file list when iterated.
# ---------------------------------------------------------------------------
set -f
for i in $(seq 0 $((${#dir_names[@]} - 1))); do
  name="${dir_names[$i]}"
  srcs="${dir_srcs[$i]}"
  for tok in $srcs; do
    if [[ "$tok" == "*" ]]; then
      fail "$name contains bare '*' wildcard — too permissive; restrict to specific origins"
    fi
  done
done
set +f

# ---------------------------------------------------------------------------
# Rule 4: script-src must not include the `data:` scheme (data: URLs can
#         carry arbitrary JS).
# ---------------------------------------------------------------------------
script_srcs_only="$(get_sources "script-src" || echo "")"
if [[ -n "$script_srcs_only" ]]; then
  set -f
  for tok in $script_srcs_only; do
    if [[ "$tok" == "data:" ]]; then
      fail "script-src allows 'data:' scheme — data: URLs can deliver inline JS"
    fi
  done
  set +f
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
if [[ "$failed" -eq 0 ]]; then
  echo "OK: CSP restrictive — ${#directives[@]} directive(s), script-src clean, no wildcards"
  exit 0
fi
echo "      Fix: see https://content-security-policy.com/ for guidance, then edit" >&2
echo "      \"app.security.csp\" in $CONF and re-run this check." >&2
exit 1
