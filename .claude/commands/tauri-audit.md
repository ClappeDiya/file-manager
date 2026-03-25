# Tauri Application Completion Audit

You are a skeptical external auditor for a **Tauri 2 + Rust + React/TypeScript** desktop application. Your default for every item is **FAIL**. Only mark PASS with file evidence.

## Evidence Rules
For every PASS, provide:
- **E1** File path(s)
- **E2** Line range or grep pattern used
- **E3** 1-2 verbatim excerpts (<=25 words each)

If you cannot provide excerpts, the item is FAIL.

---

## Phase 0: Stack Detection (Do This First)

Run these commands and record results:

```bash
# Identify project structure
ls -la
find . -name "Cargo.toml" -o -name "package.json" -o -name "tauri.conf.json" | head -20

# Rust crate structure
cat src-tauri/Cargo.toml | head -20

# Frontend package info
cat package.json | head -20

# Tauri config
cat src-tauri/tauri.conf.json | head -30
```

Record:
- Rust edition and key dependencies
- Frontend framework (React/Vue/Svelte/etc)
- Bundler (Vite/Webpack/etc)
- Tauri version (v1 or v2)
- Package manager (pnpm/npm/yarn)

---

## Phase 1: Hard Gates (Must Run)

### 1A) Rust Compilation
```bash
cd src-tauri && cargo check 2>&1
```
**PASS** = zero errors, zero warnings. Any error = FAIL.

### 1B) Rust Tests
```bash
cd src-tauri && cargo test --lib 2>&1
```
Report: X passed, Y failed, Z ignored. Any failure = FAIL.

### 1C) Rust Clippy (if available)
```bash
cd src-tauri && cargo clippy --all-targets 2>&1 || echo "clippy not available"
```
Report warnings count. Critical warnings = FAIL.

### 1D) Frontend Lint
```bash
pnpm lint 2>&1 || npm run lint 2>&1
```
Must pass. If lint script missing, note and mark FAIL.

### 1E) Frontend TypeScript Check
```bash
pnpm exec tsc --noEmit 2>&1 || npx tsc --noEmit 2>&1 || echo "tsc not available"
```
If available, must pass. If not available, note "N/A".

### 1F) Frontend Build
```bash
pnpm build 2>&1 || npm run build 2>&1
```
Must succeed for PASS.

### 1G) Tauri Build (optional — slow, skip if CI handles it)
```bash
cd src-tauri && cargo build 2>&1 | tail -5
```

---

## Phase 2: Rust Module Integrity

### 2A) Module Registration
Verify every `pub mod` in `lib.rs` corresponds to an actual directory/file:
```bash
grep "^pub mod" src-tauri/src/lib.rs
ls src-tauri/src/
```
Every declared module must exist. Any missing module = FAIL.

### 2B) AppState Completeness
Verify every manager/engine declared in `AppState` struct is:
1. Imported at the top of lib.rs
2. Initialized in `initialize_app_state()`
3. Present in both the success AND fallback `AppState` construction
4. Registered via `.manage()`

```bash
grep -n "struct AppState" src-tauri/src/lib.rs
# Then read the struct fields, init function, and .manage() calls
```

### 2C) Command Registration
Verify every `#[tauri::command]` function in `commands/` is registered in `invoke_handler`:
```bash
# Count command functions
grep -r "#\[tauri::command\]" src-tauri/src/commands/ | wc -l

# Count registered commands
grep -c "::" src-tauri/src/lib.rs | head -1
# Or more precisely:
grep "invoke_handler" -A 500 src-tauri/src/lib.rs | grep "::" | wc -l
```
Any unregistered command = FAIL for that feature.

### 2D) Commands Module Index
Verify `commands/mod.rs` declares all command submodules:
```bash
grep "pub mod" src-tauri/src/commands/mod.rs
ls src-tauri/src/commands/*.rs | grep -v mod.rs
```
Every `.rs` file must have a `pub mod` entry.

---

## Phase 3: IPC Command Coverage

### 3A) Enumerate All Tauri Commands
```bash
grep -r "#\[tauri::command\]" src-tauri/src/commands/ -A 2 | grep "pub async fn\|pub fn"
```

### 3B) Enumerate Frontend Command Invocations
```bash
grep -rn "tauriInvoke\|invoke(" src/ --include="*.ts" --include="*.tsx" | grep -v node_modules
```

### 3C) Cross-Reference
For each Tauri command, verify:
- It is registered in `invoke_handler` (Phase 2C)
- It is called from at least one frontend component/store (or is legitimately backend-only)
- The argument names match between Rust and TypeScript

Commands with no frontend caller AND no backend-only justification = FAIL.

---

## Phase 4: Frontend Store <-> Backend Alignment

### 4A) Enumerate Stores
```bash
ls src/stores/*.ts
```

### 4B) Store-Command Mapping
For each store, verify:
- Every action that calls `tauriInvoke` uses a valid command name
- The TypeScript types (interfaces/discriminated unions) match Rust struct field names and enum variants
- Serde rename attributes (`rename_all = "snake_case"`) are consistent with frontend expectations

```bash
# Find all tauriInvoke calls with command names
grep -rn "tauriInvoke" src/stores/ | grep -oP '"[a-z_]+"'
```

### 4C) Type Alignment Spot-Check
Pick 3 representative data types that cross the IPC boundary. Verify:
- Rust struct field names (after serde rename) match TypeScript interface properties
- Enum variant names (after serde rename) match TypeScript discriminated union `type` values
- Optional fields use `Option<T>` in Rust and `T | null` or `T | undefined` in TypeScript

---

## Phase 5: SQLite Migration Integrity

### 5A) Migration Sequence
```bash
grep -n "version:" src-tauri/src/storage/migrations.rs | head -20
# Or
grep -n "Migration {" -A 2 src-tauri/src/storage/migrations.rs
```
Verify versions are sequential (1, 2, 3, ...) with no gaps.

### 5B) Migration Test Assertions
```bash
grep -n "assert_eq!" src-tauri/src/storage/migrations.rs
grep -n "assert_eq!" src-tauri/src/storage/repository.rs | grep -i "version\|migrat"
```
All version assertions must match the actual migration count.

### 5C) Table Existence
For each feature that stores data, verify a CREATE TABLE or ALTER TABLE exists in the migrations:
```bash
grep "CREATE TABLE" src-tauri/src/storage/migrations.rs
```

### 5D) Foreign Key Integrity
Verify REFERENCES clauses point to tables that exist:
```bash
grep "REFERENCES" src-tauri/src/storage/migrations.rs
```

---

## Phase 6: Frontend Component Integration

### 6A) Component Registration
Every panel/page component must be:
1. Imported in `file-manager.tsx` (or equivalent root)
2. Rendered conditionally or unconditionally
3. Have a toggle mechanism (toolbar button, sidebar item, or route)

```bash
grep "import.*Panel\|import.*Page" src/components/file-manager.tsx
grep "<.*Panel\|<.*Page" src/components/file-manager.tsx
```

### 6B) Toolbar/Navigation Items
```bash
grep "ALL_TOOLBAR_ITEMS\|toolbarItems\|navItems" src/components/file-manager.tsx
```
Every panel with a toolbar toggle must appear in the toolbar items array.

### 6C) Command Palette Entries
```bash
grep -c "id:" src/components/command-palette.tsx
```
Key features should have command palette entries.

### 6D) Icon Imports
```bash
grep "from \"lucide-react\"" src/components/file-manager.tsx
```
Every icon used in JSX must be imported.

---

## Phase 7: Code Quality Scans

### 7A) TODO/FIXME/Placeholder Scan
```bash
grep -rn "TODO\|FIXME\|HACK\|XXX\|TEMP\|placeholder\|stub\|coming soon" src/ src-tauri/src/ --include="*.rs" --include="*.ts" --include="*.tsx" | grep -v node_modules | grep -v target/
```
Any TODO in production logic = FAIL for that feature.

### 7B) Dead Code (Rust)
```bash
cargo check 2>&1 | grep "warning.*dead_code\|warning.*unused"
```

### 7C) Unused Imports (TypeScript)
```bash
pnpm lint 2>&1 | grep "unused" | head -20
```

### 7D) Unwrap Safety (Rust)
```bash
grep -rn "\.unwrap()" src-tauri/src/ --include="*.rs" | grep -v test | grep -v "// safe:" | wc -l
```
Report count. Unwraps in non-test code without safety comment = concern (not automatic FAIL but flag).

---

## Phase 8: Tauri Plugin & Permission Audit

### 8A) Plugin Registration
```bash
grep "\.plugin(" src-tauri/src/lib.rs
```
Every plugin must be in `Cargo.toml` dependencies.

### 8B) Tauri Permissions
```bash
cat src-tauri/capabilities/*.json 2>/dev/null || cat src-tauri/tauri.conf.json | grep -A 20 "allowlist\|permissions"
```
Verify permissions are not overly broad.

### 8C) CSP Policy
```bash
grep -i "csp\|content.security" src-tauri/tauri.conf.json
```

---

## Checklist (Default: FAIL)

### Hard Gates
| # | Gate | Status |
|---|------|--------|
| G1 | `cargo check` — 0 errors, 0 warnings | FAIL |
| G2 | `cargo test --lib` — all pass | FAIL |
| G3 | Frontend lint passes | FAIL |
| G4 | Frontend build passes | FAIL |

### Rust Module Integrity
| # | Item | Status |
|---|------|--------|
| R1 | All `pub mod` entries in lib.rs have corresponding files | FAIL |
| R2 | AppState has all managers (init + fallback + manage) | FAIL |
| R3 | All `#[tauri::command]` functions are registered | FAIL |
| R4 | commands/mod.rs declares all command submodules | FAIL |

### IPC Command Coverage
| # | Item | Status |
|---|------|--------|
| I1 | All commands registered in invoke_handler | FAIL |
| I2 | All UI-facing commands have frontend callers | FAIL |
| I3 | Argument names match between Rust and TypeScript | FAIL |

### Store <-> Backend Alignment
| # | Item | Status |
|---|------|--------|
| S1 | Every store tauriInvoke uses a valid command name | FAIL |
| S2 | TypeScript types match Rust serde output | FAIL |
| S3 | Optional field handling consistent (Option vs null) | FAIL |

### SQLite Migration Integrity
| # | Item | Status |
|---|------|--------|
| M1 | Migration versions sequential, no gaps | FAIL |
| M2 | Test assertions match migration count | FAIL |
| M3 | All feature tables exist in migrations | FAIL |
| M4 | Foreign key references are valid | FAIL |

### Frontend Component Integration
| # | Item | Status |
|---|------|--------|
| C1 | All panels imported and rendered in root | FAIL |
| C2 | All panels have toolbar/nav toggle entries | FAIL |
| C3 | Key features have command palette entries | FAIL |
| C4 | All used icons are imported | FAIL |

### Code Quality
| # | Item | Status |
|---|------|--------|
| Q1 | No TODO/FIXME in production logic | FAIL |
| Q2 | No dead code warnings (Rust) | FAIL |
| Q3 | No unused imports (TypeScript) | FAIL |
| Q4 | Unwrap usage reviewed and justified | FAIL |

### Tauri Config & Security
| # | Item | Status |
|---|------|--------|
| T1 | All plugins registered and in Cargo.toml | FAIL |
| T2 | Permissions not overly broad | FAIL |
| T3 | CSP policy configured | FAIL |

---

## Adversarial Self-Check (Mandatory)
After completing the checklist:
1. Pick the 3 items most likely to be wrong
2. Re-verify each with fresh evidence
3. If any flips PASS -> FAIL, update checklist

---

## Final Gate

**Gate Status: PASS** — only if every item is PASS and all hard gates pass.
**Gate Status: FAIL** — if any item is FAIL. List blockers and implement fixes, then re-audit.
