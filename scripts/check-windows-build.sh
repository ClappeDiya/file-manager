#!/usr/bin/env bash
# check-windows-build.sh — type-check + lint the Windows-only code paths.
#
# Why this exists: `#[cfg(windows)]` / `#[cfg(target_os = "windows")]` code
# is invisible to `cargo check` on macOS and Linux, which is where local CI
# runs. Two Windows-only compile errors and a broken drive listing once
# shipped that way. Cross-checking against the Windows target catches them
# without a Windows machine.
#
# Needs the target (`rustup target add x86_64-pc-windows-gnu`) and, for
# crates with C build scripts (ring, sqlite), a MinGW C compiler
# (`x86_64-w64-mingw32-gcc`). Skipped with a warning when either is missing,
# so machines without the toolchain aren't blocked.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="x86_64-pc-windows-gnu"

if ! rustup target list --installed 2>/dev/null | grep -qx "$TARGET"; then
  echo "WARN: Rust target $TARGET not installed — skipping Windows check. Install with: rustup target add $TARGET"
  exit 0
fi
if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
  echo "WARN: x86_64-w64-mingw32-gcc not found — skipping Windows check. Install MinGW-w64 (apt: gcc-mingw-w64-x86-64, brew: mingw-w64)."
  exit 0
fi

fail=0
for crate in crates/ufop-core src-tauri; do
  echo "--- $crate ($TARGET)"
  (cd "$ROOT/$crate" && cargo clippy --all-targets --target "$TARGET" -- -D warnings) || fail=1
done
exit $fail
