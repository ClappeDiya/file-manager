# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Unified File Operations Platform (UFOP) — a cross-platform desktop file manager with transfer, sync, and governance capabilities. Three integrated components: Tauri desktop app (React + Rust), Next.js admin console, and CLI.

## Commands

### Desktop App (Tauri + React/Vite)
```bash
pnpm dev              # Start Vite dev server (port 1420) — frontend only, no Tauri
pnpm tauri:dev        # Start full Tauri desktop app in dev mode (Rust + React)
pnpm build            # Production build (frontend only)
pnpm tauri:build      # Build native desktop binary
pnpm lint             # ESLint on src/
pnpm format           # Prettier on src/
pnpm test             # Vitest (watch mode)
pnpm test:watch       # Vitest watch mode (same as above)
```

### Rust Backend
```bash
cd src-tauri
cargo check           # Fast compilation check (no codegen)
cargo test --lib      # Run all library tests (~857 tests)
cargo test --lib automation  # Run tests matching "automation"
cargo clippy          # Lint (if installed)
```

### Admin Console (Next.js)
```bash
pnpm admin:dev        # Start admin console on port 3001
```

### Package Manager
pnpm 10.27.0. Monorepo with workspaces: root, `packages/design-tokens`, `packages/ui-components`, `admin`.

## Architecture

### IPC Boundary (Most Important Pattern)
Frontend communicates with Rust via Tauri IPC commands. Every backend operation goes through this boundary:

- **Rust side**: `#[tauri::command]` functions in `src-tauri/src/commands/*.rs`, registered in `lib.rs` `invoke_handler`
- **Frontend side**: `tauriInvoke<T>("command_name", { args })` from `src/hooks/use-tauri.ts`
- **Fallback**: `tauriInvokeSafe<T>()` never throws — returns fallback value when outside Tauri (enables browser testing)
- **Detection**: `isTauriAvailable()` checks for `__TAURI__` in window

### Rust Backend (`src-tauri/src/`)

**Module registration**: Every engine module must be declared as `pub mod` in `lib.rs`.

**AppState pattern**: All managers are initialized in `initialize_app_state()`, stored in an `AppState` struct, then individually registered via `.manage()` on the Tauri builder. There are TWO init paths — the normal path and an in-memory fallback — both must include every manager.

**Command modules** (`commands/`): ~30 modules, one per feature area. Every `.rs` file needs a `pub mod` entry in `commands/mod.rs` and its commands registered in the `invoke_handler` macro in `lib.rs`.

**Error handling** (`core/error.rs`): `AppError` enum with `{ message, advice }` fields. Every error variant carries a user-facing explanation and suggested fix. Stack traces never leak to frontend. The `Internal` variant sanitizes messages.

**Traits** (`core/traits.rs`): Async traits using `BoxFuture<'_, Result<T, AppError>>` for dependency injection. Key traits: `FsOperations`, `TransferOperations`, `SyncOperations`, `StorageOperations`.

**Database** (`storage/pool.rs`): Single SQLite connection wrapped in `Arc<Mutex<Connection>>`. All DB operations go through `pool.execute(|conn| { ... })` which runs on `tokio::task::spawn_blocking`. WAL mode, foreign keys enabled, 5s busy timeout.

**Migrations** (`storage/migrations.rs`): Versioned SQL scripts in `all_migrations()` vector. When adding a migration, update version count in test assertions (both `migrations.rs` and `repository.rs`).

### Frontend (`src/`)

**State management**: Zustand stores in `src/stores/`. Stores using `persist()` middleware save to localStorage. Tests must reset store state in `beforeEach`.

**Component hierarchy**: `file-manager.tsx` is the root orchestrator — imports and renders all panels (AI, Automation, Terminal), sidebar, dual-pane layout, command palette, and context menus.

**Path aliases**: `@/` maps to `./src/`, `@ufop/design-tokens` and `@ufop/ui-components` map to workspace packages.

**UI library**: shadcn/ui + React Aria + Tailwind CSS. Icons from lucide-react. Shared design tokens via `@ufop/design-tokens`.

### Adding a New Feature (End-to-End Checklist)

1. **Rust engine**: Create `src-tauri/src/{feature}_engine/mod.rs` with data structures and manager
2. **Commands**: Create `src-tauri/src/commands/{feature}_commands.rs` with `#[tauri::command]` functions
3. **Wire commands**: Add `pub mod` in `commands/mod.rs`, import in `lib.rs`, add to `invoke_handler`
4. **AppState**: Add manager field to `AppState`, initialize in BOTH init paths, add `.manage()` call
5. **Migration**: Add new version to `all_migrations()`, update test assertions for version count
6. **Frontend store**: Create `src/stores/{feature}-store.ts` with Zustand + TypeScript types matching Rust serde output
7. **Component**: Create `src/components/{feature}-panel.tsx`, import and render in `file-manager.tsx`
8. **Toolbar**: Add entry to `ALL_TOOLBAR_ITEMS` array, add toggle button in toolbar JSX
9. **Command palette**: Add entries in `command-palette.tsx` (`getDefaultCommands` actions object)

### Key Conventions

- Rust enums use `#[serde(tag = "type", rename_all = "snake_case")]` — TypeScript discriminated unions must match these exact snake_case `type` values
- `Option<T>` in Rust serializes as `T | null` in JSON — use `T | null` in TypeScript (not `T | undefined`)
- All async Rust errors return `Result<T, AppError>` — never use `.unwrap()` in command handlers
- Toolbar items array (`ALL_TOOLBAR_ITEMS`) controls the toolbar customizer UI — every panel toggle needs an entry
- Frontend works outside Tauri with fallback data — always provide a fallback parameter to `tauriInvoke`

### Connector Protocol Pattern (`connectors/`)

All 17 protocol connectors implement the `Connector` trait: `connect()`, `disconnect()`, `list_remote()`, `is_connected()`. New connectors follow this pattern and register in `ConnectorRegistry`.

### Transfer Engine Three-Layer Architecture

Layer 1 (throughput): Worker pool + pipeline. Layer 2 (integrity): xxHash3/SHA-256 checksums + Merkle tree. Layer 3 (crash recovery): Journal + chunk bitmap + atomic rename. `TransferManager::recover_from_journal()` runs on startup.

## Testing

- Always add a regression test when fixing a bug — reproduce the failure first, then fix.
- Account for DRF serializer behavior when writing assertions (e.g. `EmailField` being dropped from payloads) anywhere a Django/DRF backend is exercised.

## Shipping Workflow

When asked to commit / push / PR / merge:

1. Branch off the work (never commit directly to `master`).
2. Exclude screenshot / report / artifact files from commits.
3. Open a PR, then merge and sync `master`.
4. If the original branch hits conflicts, prefer a clean cherry-picked PR.

## Deployment Verification

**Never declare a deploy successful on green status alone — success means every route is verified clean.** See `pnpm`/`cargo`/`admin:dev` commands above for the build surfaces this gates.

Run the deploy-verify-heal pipeline on every production deploy:

1. **Pre-flight** — check for container-name conflicts and stale Traefik/proxy config (the usual causes of a green-but-broken deploy) and resolve them automatically before promoting.
2. **Deploy** the latest commit.
3. **Verify** — once live, drive a browser (Playwright / Chrome DevTools MCP) across **all** routes; assert zero console errors and no 502s.
4. **Heal** — on any failed route, diagnose the root cause (e.g. stale Traefik config), fix, redeploy, and re-verify.
5. **Clean up** — prune reclaimable Docker space safely.
6. **Report** — per-route screenshots with pass/fail evidence; surface every failure explicitly.

## Recurring Improvement Loops

Periodically re-run quality gates against the live codebase rather than waiting for failures: `engineering-audit` / `tauri-audit` for completion and production-readiness checks, `/code-review` for diff-level correctness, and the Deployment Verification pipeline above after each release. Treat findings as the next loop's work-list.
