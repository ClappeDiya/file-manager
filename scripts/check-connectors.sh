#!/usr/bin/env bash
# check-connectors.sh — verify every Connector trait implementation is wired
# into the ConnectorRegistry.
#
# Why this exists:
#   CLAUDE.md flags this pattern: "All 17 protocol connectors implement the
#   `Connector` trait... New connectors follow this pattern and register in
#   `ConnectorRegistry`." The compiler will happily build a new `impl Connector
#   for FooConnector` without complaint, but if you forget to add a
#   `registry.register("foo", Arc::new(FooConnector::new()))` call in
#   `ConnectorRegistry::new()`, the connector is invisible at runtime — the
#   frontend's protocol → connector lookup silently returns None and the user
#   gets "unsupported protocol" with no other clue. Same DNA as the AppState,
#   migration count, and Zustand persist-key checks.
#
# What it checks:
#   1. Every struct that has `impl (super::)?Connector for <Name>` in any file
#      under `src-tauri/src/connectors/*.rs` (excluding `mod.rs`) has at least
#      one `Arc::new(<Name>::new())` callsite inside `connectors/mod.rs`.
#   2. Every `.rs` file under `connectors/` (excluding `mod.rs`) that contains
#      a Connector impl is declared as `pub mod <stem>;` in `mod.rs`. Compiler
#      already enforces this for files that are USED, but a file that exists
#      on disk but is never `pub mod`-d compiles to nothing — this catches the
#      orphan-file case.
#
# What it does NOT check:
#   - That the protocol-string key (e.g. "sftp") matches anything sensible —
#     that's a runtime concern between the frontend ConnectionProtocol enum
#     and Rust ConnectionProtocol::as_str(); we have IPC contract coverage
#     for that path already.
#   - That a single struct can legitimately be registered under multiple
#     protocol keys (e.g. FtpConnector handles both "ftp" and "ftps") — we
#     only require ≥1 registration, not exact count.
#
# Pure-bash + awk, no jq / python. Tokenizer-based: strips block comments,
# line comments, and string contents before pattern-matching, so source text
# that mentions `impl Connector for ...` inside a doc-comment or string
# literal cannot trigger a spurious finding (same false-positive class iter 10
# fought).
#
# Usage:
#   ./scripts/check-connectors.sh         # verify; non-zero on mismatch
#   ./scripts/check-connectors.sh --list  # debug: print parsed sets

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

CONN_DIR="src-tauri/src/connectors"
MOD_FILE="$CONN_DIR/mod.rs"

LIST=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --list)  LIST=true; shift ;;
    -h|--help)
      sed -n '2,45p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "Unknown arg: $1 (try --help)"; exit 2 ;;
  esac
done

if [[ ! -d "$CONN_DIR" ]]; then
  echo "ERROR: connectors directory not found: $CONN_DIR" >&2
  exit 1
fi
if [[ ! -f "$MOD_FILE" ]]; then
  echo "ERROR: $MOD_FILE not found" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Tokenizer: emit each file's source with block/line comments and string
# contents (single, double, raw, byte, byte-string) stripped — so subsequent
# regex matching only sees executable Rust code.
#
# Rust string forms handled:
#   "..."  (double-quoted, with \" / \\ / \n etc. escapes)
#   '...'  (char literal — including lifetime 'a which has no closing ')
#   b"..." (byte string)
#   r"..." / r#"..."# / r##"..."## (raw string, balanced hash count)
#   br"..." / br#"..."#
#
# For simplicity we replace the BODY of strings with empty, keeping the
# delimiters so the surrounding tokens still parse. Lifetimes like 'a are
# benign — we leave them alone if no closing ' appears within a short window.
#
# This is the same defensive lesson as iter 10: a single grep over the raw
# source can be defeated by a description string that happens to contain the
# pattern we're searching for.
# ---------------------------------------------------------------------------
AWK_STRIP="$(mktemp -t check-connectors.XXXXXX.awk)"
trap 'rm -f "$AWK_STRIP"' EXIT

cat > "$AWK_STRIP" <<'AWK_EOF'
# Strip Rust comments and string literals. Output one line per input line.
BEGIN {
  in_block = 0    # inside /* ... */
  in_str = ""     # current string delimiter: "  '  or raw-hash count e.g. r0 r1 r2
}
{
  line = $0
  out = ""
  i = 1
  n = length(line)
  while (i <= n) {
    c = substr(line, i, 1)

    # Inside a block comment
    if (in_block) {
      if (c == "*" && substr(line, i + 1, 1) == "/") {
        in_block = 0
        i += 2
        continue
      }
      i++
      continue
    }

    # Inside a double-quoted string
    if (in_str == "\"") {
      if (c == "\\" && i + 1 <= n) { i += 2; continue }
      if (c == "\"") { out = out "\""; in_str = ""; i++; continue }
      i++; continue
    }

    # Inside a char literal '
    if (in_str == "'") {
      if (c == "\\" && i + 1 <= n) { i += 2; continue }
      if (c == "'") { out = out "'"; in_str = ""; i++; continue }
      i++; continue
    }

    # Inside a raw string r#..."..."#... — in_str = "r<N>"
    if (substr(in_str, 1, 1) == "r") {
      hashes = substr(in_str, 2) + 0
      if (c == "\"") {
        end_hashes = ""
        for (k = 1; k <= hashes; k++) end_hashes = end_hashes "#"
        if (substr(line, i + 1, hashes) == end_hashes) {
          out = out "\"" end_hashes
          in_str = ""
          i += 1 + hashes
          continue
        }
      }
      i++; continue
    }

    # Not in any string — look for openers
    if (c == "/" && substr(line, i + 1, 1) == "*") {
      in_block = 1
      i += 2
      continue
    }
    if (c == "/" && substr(line, i + 1, 1) == "/") {
      # Rest of line is a line comment
      break
    }

    # Raw / byte / byte-raw string opener: r"  r#"  br"  b"  br#"
    # Lookbehind: only treat as opener if prev char is not an identifier char,
    # to avoid false-positive on identifiers like `foo_r` immediately followed
    # by `"x"` (unlikely but cheap to guard).
    prev = (i == 1 ? " " : substr(line, i - 1, 1))
    is_ident_prev = (prev ~ /[A-Za-z0-9_]/)

    if (!is_ident_prev && (c == "r" || c == "b")) {
      # Try br" / br# first
      head2 = substr(line, i, 2)
      head3 = substr(line, i, 3)
      if (head3 ~ /^br[#"]/) {
        # br"..."  or  br#..."..."#
        j = i + 2
        h = 0
        while (substr(line, j, 1) == "#") { h++; j++ }
        if (substr(line, j, 1) == "\"") {
          out = out "br"
          for (k = 0; k < h; k++) out = out "#"
          out = out "\""
          in_str = "r" h
          i = j + 1
          continue
        }
      }
      if (c == "r") {
        j = i + 1
        h = 0
        while (substr(line, j, 1) == "#") { h++; j++ }
        if (substr(line, j, 1) == "\"") {
          out = out "r"
          for (k = 0; k < h; k++) out = out "#"
          out = out "\""
          in_str = "r" h
          i = j + 1
          continue
        }
      }
      if (c == "b" && substr(line, i + 1, 1) == "\"") {
        out = out "b\""
        in_str = "\""
        i += 2
        continue
      }
    }

    if (c == "\"") {
      out = out "\""
      in_str = "\""
      i++
      continue
    }
    if (c == "'") {
      # Distinguish lifetime ('a, 'static) from char literal ('x', '\n').
      # A lifetime is: ' followed by ident char(s) NOT followed by '.
      rest = substr(line, i + 1)
      if (rest ~ /^[A-Za-z_][A-Za-z0-9_]*[^'A-Za-z0-9_\\]/ \
          || rest ~ /^[A-Za-z_][A-Za-z0-9_]*$/) {
        # Lifetime — emit verbatim, do not enter string mode
        out = out c
        i++
        continue
      }
      out = out "'"
      in_str = "'"
      i++
      continue
    }

    out = out c
    i++
  }
  print out
}
AWK_EOF

strip_rust() {
  awk -f "$AWK_STRIP" "$1"
}

# ---------------------------------------------------------------------------
# 1. Scan every connector .rs file (except mod.rs) for Connector impls.
#    Collect: struct names, and which file each was declared in.
# ---------------------------------------------------------------------------
impl_structs=()
impl_files=()
impl_mods=()

for f in "$CONN_DIR"/*.rs; do
  base="$(basename "$f")"
  [[ "$base" == "mod.rs" ]] && continue
  stem="${base%.rs}"

  # Extract all `impl <path::>Connector for <Name>` lines from stripped source.
  # Path prefix accepts any sequence of `ident::` (covers `super::`,
  # `super::super::`, `crate::connectors::`, etc.) so impls inside nested
  # modules are detected too. The trade-off: a different trait literally
  # named `Connector` in some other crate would also match — accepted as a
  # rare-and-easily-spotted false positive vs. the more dangerous false
  # negative of an unregistered connector hiding in a submodule.
  while IFS= read -r struct_name; do
    [[ -z "$struct_name" ]] && continue
    impl_structs+=("$struct_name")
    impl_files+=("$f")
    impl_mods+=("$stem")
  done < <(strip_rust "$f" \
    | grep -oE 'impl(<[^>]+>)?[[:space:]]+([A-Za-z_][A-Za-z0-9_]*::)*Connector[[:space:]]+for[[:space:]]+[A-Za-z_][A-Za-z0-9_]*' \
    | awk '{ print $NF }')
done

if [[ "${#impl_structs[@]}" -eq 0 ]]; then
  echo "ERROR: parsed zero Connector impls — extractor likely broken" >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# 2. Scan mod.rs for registry registrations and pub mod declarations.
# ---------------------------------------------------------------------------
stripped_mod="$(strip_rust "$MOD_FILE")"

# Set of structs that appear inside `Arc::new(<Name>...)` in mod.rs.
# Comma-delimited for cheap membership check.
#
# We capture any identifier directly after `Arc::new(` regardless of what
# follows it — so `Foo::new()`, `Foo::with_config()`, `Foo::default()`,
# `Foo { field: 1 }`, and bare-tuple `Foo()` are all recognised as a
# registration of `Foo`. Restricting to `::new(` was an over-fit that
# falsely failed any connector using a non-default constructor.
registered_csv=","
while IFS= read -r name; do
  [[ -z "$name" ]] && continue
  registered_csv="${registered_csv}${name},"
done < <(echo "$stripped_mod" \
  | grep -oE 'Arc::new\([[:space:]]*[A-Za-z_][A-Za-z0-9_]*' \
  | sed -E 's/^Arc::new\([[:space:]]*//')

# Set of pub-mod-declared stems inside mod.rs.
# Splitting on `;` first means multiple declarations on one line
# (`pub mod a; pub mod b;` — rare but valid Rust) are all captured.
pubmod_csv=","
while IFS= read -r m; do
  [[ -z "$m" ]] && continue
  pubmod_csv="${pubmod_csv}${m},"
done < <(echo "$stripped_mod" \
  | tr ';' '\n' \
  | grep -oE 'pub[[:space:]]+mod[[:space:]]+[A-Za-z_][A-Za-z0-9_]*' \
  | awk '{ print $NF }')

# ---------------------------------------------------------------------------
# 3. Report and check.
# ---------------------------------------------------------------------------
if [[ "$LIST" == true ]]; then
  echo "Connector impls found (${#impl_structs[@]}):"
  for i in "${!impl_structs[@]}"; do
    s="${impl_structs[$i]}"
    m="${impl_mods[$i]}"
    reg="missing"
    [[ "$registered_csv" == *",$s,"* ]] && reg="registered"
    pm="missing"
    [[ "$pubmod_csv" == *",$m,"* ]] && pm="pub mod"
    printf '  %-25s  %-20s  %-12s  %s\n' "$s" "(file: $m.rs)" "$reg" "$pm"
  done
fi

failed=0
fail() { echo "FAIL: $*" >&2; failed=$((failed + 1)); }

for i in "${!impl_structs[@]}"; do
  s="${impl_structs[$i]}"
  m="${impl_mods[$i]}"
  if [[ "$registered_csv" != *",$s,"* ]]; then
    fail "$s (in ${impl_mods[$i]}.rs) implements Connector but is not registered in ConnectorRegistry — add \`registry.register(\"<protocol>\", Arc::new($s::new()))\` to $MOD_FILE"
  fi
  if [[ "$pubmod_csv" != *",$m,"* ]]; then
    fail "${m}.rs contains a Connector impl but is not declared as \`pub mod $m;\` in $MOD_FILE"
  fi
done

if [[ "$failed" -eq 0 ]]; then
  echo "OK: ${#impl_structs[@]} Connector impl(s), all registered in ConnectorRegistry"
  exit 0
fi
exit 1
