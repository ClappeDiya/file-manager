# Contributing & Development

## Project layout

```
FileManager/
├── src/                              ← Tauri frontend (React + TypeScript + Vite)
├── src-tauri/                        ← Rust backend (Tauri 2 + engines + ~30 command modules)
├── admin/                            ← Next.js admin console + customer portal
├── marketing/                        ← Next.js marketing site (filemanager.clappe.com)
├── cli/                              ← Rust CLI binary
├── packages/                         ← Shared workspace packages
│   ├── design-tokens/                ← Design tokens consumed by all UI apps
│   └── ui-components/                ← Shared React component library
├── docs/                             ← Product + technical docs (rendered via Nextra)
│   ├── PRD-V5-Definitive.md          ← Master PRD
│   └── AI-Agent-Governance-Addendum.md
├── ai-agents/                        ← AI agent configurations (development tooling)
│   ├── global-policy.yaml            ← Shared governance rules
│   ├── agent-index.yaml              ← Agent registry
│   ├── core/                         ← 11 core agents
│   ├── support/                      ← 8 support agents
│   └── workflows/                    ← Task bundle templates
└── planning/                         ← 42-sprint delivery plan
```

## Tech stack

- **Desktop:** Tauri 2 + Rust (engines, command handlers, SQLite + WAL) + React 19 + TypeScript 5 + Vite 6
- **State:** Zustand stores in `src/stores/` with `persist()` middleware
- **UI:** shadcn/ui + React Aria + Tailwind v4 + lucide-react icons
- **Data:** TanStack Query, TanStack Table, TanStack Virtual
- **Admin/Marketing:** Next.js 15 App Router + TypeScript + Tailwind v4
- **Package manager:** pnpm 10.27.0 (workspaces)

## Common commands

### Desktop app
```bash
pnpm dev              # Vite dev server (port 1420) — frontend only
pnpm tauri:dev        # Full Tauri desktop in dev mode (Rust + React)
pnpm tauri:build      # Native installer build
pnpm lint             # ESLint on src/
pnpm format           # Prettier on src/
pnpm test             # Vitest
```

### Rust backend
```bash
cd src-tauri
cargo check           # Fast compilation check
cargo test --lib      # ~733 library tests
cargo clippy          # Lint
```

### Admin console
```bash
pnpm admin:dev        # Port 3001
pnpm admin:build
```

### Marketing site
```bash
pnpm --filter marketing dev    # Port 3000
pnpm --filter marketing build
```

### Pre-commit / pre-release gate
```bash
pnpm verify           # full gate: lint + typecheck + ipc-verify + appstate + vitest + cargo + cargo test + vite build
pnpm verify:fast      # skip cargo test + vite build (fast-feedback loop, ≈25s)
```
`scripts/verify.sh` runs every stage in one pass, continues on failure, prints a status table, and exits non-zero if any stage failed. The `ipc-verify` stage checks both directions of the Tauri IPC contract (Rust `#[tauri::command]` ↔ `invoke_handler!`, and frontend `tauriInvoke<T>("name")` ↔ real commands); the `appstate` stage checks `struct AppState` field ↔ `.manage()` wiring; the `migrations` stage checks `all_migrations()` count ↔ test assertions; the `stores` stage checks Zustand `persist()` keys are unique and `ufop-*` namespaced; the `connectors` stage checks every `impl Connector` struct is registered in `ConnectorRegistry::new()`; the `unwraps` stage rejects `.unwrap()` / `.expect(` inside `#[tauri::command]` bodies (a panic there crashes the IPC handler); the `toolbar` stage keeps `ALL_TOOLBAR_ITEMS` ↔ `toolbarItems.includes(...)` in lockstep so the customizer matches the actual toggles; the `plugins` stage rejects any `tauri-plugin-*` dependency that has no `.plugin()` call in `lib.rs` (a dormant plugin compiles cleanly but its APIs never fire); the `pm` stage rejects competing lockfiles (`package-lock.json` / `yarn.lock` / `bun.lock*`) and missing/mismatched `packageManager` so a stray `npm install` cannot silently shadow pnpm's dependency tree; the `artifacts` stage rejects any tracked build-cache directory (`node_modules`, `dist`, `target`, `.next`, etc.), any OS/log noise (`.DS_Store`, `*.log`), and any raw `.env*` that is not a `.example` template — `git rm --cached` is the only fix once tracked, so catching at the gate avoids history-scrubbing later; the `version` stage rejects any drift across the six release manifests (root `package.json`, `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml`, `cli/Cargo.toml`, `admin/package.json`, `marketing/package.json`) — a partial bump otherwise ships `FileManager-1.0.0.dmg` that reports its internal version as `0.9.0`; the `tauri-vers` stage rejects any drift across the Tauri JS package set (`@tauri-apps/api`, `@tauri-apps/cli`, `@tauri-apps/plugin-*`) and the Rust crate set (`tauri`, `tauri-build`, `tauri-plugin-*`) — bumping JS to major v3 while leaving Rust on v2 silently breaks the IPC wire format; the `hygiene` stage rejects any `scripts/*.sh` that lacks the portable shebang, a `set -uo` (or `-euo`) `pipefail` directive, or the executable bit, so verify-gate scripts cannot silently swallow errors; the `changelog` stage rejects shipping a release without a matching `## [X.Y.Z]` section in `CHANGELOG.md` — bumping the version through the iter-17 manifest sync without telling users what changed is the failure mode it catches; the `gha` stage rejects any active `*.yml` / `*.yaml` in `.github/workflows/` at the repo root, enforcing the project's local-CI/CD + GHCR policy and preventing a re-enabled Actions runner from racing the local pipeline; the `audit` stage runs `pnpm audit --audit-level=critical --prod` and fails on any critical-severity CVE in the production dep tree (network failures are tolerated as warnings, not gate-blocking, so offline development continues to work). All sixteen stages run in `--fast` mode (sub-4s total) since they catch silent runtime footguns that compile cleanly. GitHub Actions are intentionally disabled (see `.github/workflows/*.yml.disabled`), so this is the authoritative local gate before `release-local.sh`.

### IPC command index
```bash
pnpm docs:ipc         # regenerate docs/ipc-commands.md from #[tauri::command] declarations
pnpm docs:ipc:check   # exit non-zero if the index is stale (use after adding a command)
pnpm docs:ipc:verify  # exit non-zero if either IPC contract direction is broken (see below)
```
`docs/ipc-commands.md` is the searchable index of every Tauri command callable from the frontend. Re-run `pnpm docs:ipc` whenever you add or rename a `#[tauri::command]` so the index stays in sync with `src-tauri/src/commands/`.

`pnpm docs:ipc:verify` checks both sides of the IPC contract in one pass:
- **handler direction** — every `#[tauri::command]` in `src-tauri/src/commands/*.rs` must be wired into the `invoke_handler!` macro in `src-tauri/src/lib.rs`. Otherwise the function compiles but the frontend's `tauriInvoke<T>("name")` fails at runtime with "command not found".
- **frontend direction** — every `tauriInvoke<T>("name", …)` / `tauriInvokeSafe<T>("name", …)` callsite in `src/` (including multi-line form) must reference a real registered command. Catches typos that compile and type-check but blow up only at runtime. Dynamic dispatch (variable-name argument) is intentionally not validated.

Run the directions individually with `bash scripts/dump-ipc-commands.sh --verify-handler` or `--verify-frontend`.

### AppState wiring check
```bash
pnpm appstate:verify  # exit non-zero if any AppState field is not .manage()-d
```
Every field in `struct AppState { ... }` in `src-tauri/src/lib.rs` must have a corresponding `.manage(app_state.<field>)` call on the Tauri builder, otherwise commands using `tauri::State<T>` for that type will compile but panic at runtime. The struct literal is compiler-checked (you can't forget a field in the init paths), but the chained `.manage()` registrations are not — this script closes that gap. Use `bash scripts/check-appstate.sh --list` to inspect the parsed sets.

### Migration count check
```bash
pnpm migrations:check  # exit non-zero if the migration count is out of sync
```
The `all_migrations()` vector in `src-tauri/src/storage/migrations.rs` is the source of truth for how many migrations exist. Two tests assert the resulting count: `test_current_version` in `migrations.rs` and `test_repository_creation` in `repository.rs`. When you add a migration, you have to bump *both* assertions — and the cargo error if you forget is a confusing `assert_eq!(11, 12)` instead of "you forgot to bump the count". This script reads the `Vec<Migration>` (multi-line and inline styles both supported), pulls both assertion values, and verifies all three match. It also flags duplicate version numbers and gaps in the sequence (often the trace of a sloppy merge conflict). Pure-bash, sub-second; use `bash scripts/check-migrations.sh --list` to see the parsed values.

### Zustand persist-key check
```bash
pnpm stores:check  # exit non-zero if any persist() key collides or breaks convention
```
Every Zustand store in `src/stores/` that wraps its state with `persist()` writes to `localStorage` under the `name:` key in its config object. Two stores sharing a key silently overwrite each other's persisted state — the failure mode is invisible until a real user reloads the app and gets a mix of two corrupt slices. This script enforces: exactly one `persist()` per store file, every `persist()` declares a `name:` literal, all keys are unique across the directory, and all keys start with the `ufop-` prefix (so they cannot collide with a third-party library that picks generic localStorage names). Pure-bash, sub-second; use `bash scripts/check-store-keys.sh --list` to see the parsed name → file map.

### Connector registry check
```bash
pnpm connectors:check  # exit non-zero if any Connector impl is not registered
```
Every struct that declares `impl (super::)?Connector for X` in `src-tauri/src/connectors/*.rs` must (a) have its file declared as `pub mod` in `connectors/mod.rs` and (b) be instantiated via `Arc::new(X::new())` inside `ConnectorRegistry::new()`. The compiler accepts a stranded connector silently — the protocol lookup just returns `None` at runtime and the user gets "unsupported protocol" with no other clue. The script uses an awk tokenizer to strip block/line comments and string contents before pattern-matching, so doc-comments or descriptions that mention `impl Connector` cannot fire a spurious finding. Pure-bash, sub-second; use `bash scripts/check-connectors.sh --list` to see the parsed impl → registration map. A struct registered under multiple protocols (e.g. `FtpConnector` for both `ftp` and `ftps`) is accepted.

### Command-handler panic check
```bash
pnpm unwraps:check  # exit non-zero if any #[tauri::command] body contains .unwrap() / .expect(
```
Every `#[tauri::command]` function is on the IPC boundary: a panic inside it crashes the Tauri runtime handler, the frontend receives an opaque error, and the user's session is poisoned for that subsystem. CLAUDE.md mandates: "All async Rust errors return `Result<T, AppError>` — never use `.unwrap()` in command handlers". The check parses each `src-tauri/src/commands/*.rs`, identifies every `#[tauri::command]` function body, and flags `.unwrap()` or `.expect(` calls inside it. `.unwrap_or(...)`, `.unwrap_or_default()`, and `.unwrap_or_else(...)` are allowed (they never panic). Calls inside `#[cfg(test)]` modules are skipped — unwrap is idiomatic for test setup. Pure-bash + awk tokenizer, sub-second; use `bash scripts/check-command-unwraps.sh --list` to see counts. To fix a flagged call, swap in `.map_err(|e| AppError::internal(e))?` or `.ok_or_else(|| AppError::not_found(...))?`.

### Toolbar items consistency check
```bash
pnpm toolbar:check  # exit non-zero if ALL_TOOLBAR_ITEMS drifts from toolbarItems.includes() use
```
The toolbar customizer renders one row per entry in `ALL_TOOLBAR_ITEMS` (in `src/components/file-manager.tsx`); each toolbar toggle in the JSX is gated by `toolbarItems.includes("<id>")`. Two failure modes the compiler can't catch: a stale entry in `ALL_TOOLBAR_ITEMS` whose id is no longer used anywhere (a dead row in the customizer that does nothing) and a `toolbarItems.includes("foo")` gate whose id is missing from the array (a panel toggle the customizer cannot hide or show). The check uses a tiny TS/JSX tokenizer to strip block/line comments and skip string contents before pattern-matching; nested `meta: { id: "x" }` keys inside an entry object are ignored (only depth-1 `id:` keys count); multi-line `includes()` calls are supported. Pure-bash + awk, sub-second; use `bash scripts/check-toolbar-items.sh --list` to see both parsed sets.

### Tauri plugin registration check
```bash
pnpm plugins:check  # exit non-zero if any Cargo tauri-plugin-* has no .plugin() call
```
A Tauri plugin (`tauri-plugin-shell`, `tauri-plugin-updater`, etc.) only activates after `tauri::Builder::default()` is chained with `.plugin(tauri_plugin_X::init())`. Forgetting that call leaves the crate compiled into the binary but **dormant** — its commands and events never fire, and the only signal at runtime is opaque "command not found" / "permission denied" errors when the frontend reaches for the plugin's APIs. The check parses every `tauri-plugin-*` entry from `[dependencies]` in `src-tauri/Cargo.toml` (skipping `[dev-dependencies]`, `[build-dependencies]`, commented-out lines, and same-line `# ...` annotations) and verifies a matching `.plugin(tauri_plugin_X::...)` call exists in `src-tauri/src/lib.rs` (both `::init()` and `::Builder::new().build()` forms accepted; kebab→snake name normalization handled). Pure-bash + awk, sub-second; use `bash scripts/check-tauri-plugins.sh --list` to see both parsed sets. The reverse direction (every `.plugin()` call has a Cargo entry) is already enforced by `cargo check` — an unresolved `tauri_plugin_X::` path fails to link.

### Package-manager hygiene check
```bash
pnpm pm:check  # exit non-zero on competing lockfile or missing/wrong packageManager
```
This is a pnpm 10 workspace (root + `admin/` + `marketing/` + `packages/*`). The check enforces two invariants: (1) `pnpm-lock.yaml` is the **only** lockfile in the tree — `package-lock.json`, `yarn.lock`, `bun.lockb`, or `bun.lock` anywhere outside `node_modules/` / `.next/` / `target/` / `dist/` / `.git/` / `.vite/` fails, because pnpm and npm/yarn compute different dependency trees from the same `package.json` and a parallel lockfile silently drifts the resolution; (2) root `package.json` declares `"packageManager": "pnpm@<version>"` (Corepack form, hash suffix accepted) so contributors and CI use the same pnpm version. Pure-bash, sub-second; use `bash scripts/check-package-manager.sh --list` to see the parsed values.

### Tracked build-artifact check
```bash
pnpm artifacts:check  # exit non-zero if a build/cache/env file is tracked by git
```
`.gitignore` is a *suggestion* — once a file is tracked it stays tracked until `git rm --cached` is run, and a stray `git add -f target/` can silently land build artifacts (or worse, signing material) in history. The check uses `git ls-files` as the source of truth and fails on three categories: (1) any path containing a build/cache segment (`node_modules`, `target`, `dist`, `.next`, `.vite`, `.turbo`, `.svelte-kit`, `coverage`, `out`, `build` — matched as a path component, so `distance/` and `outputs/` are NOT flagged); (2) OS/log noise (`.DS_Store`, `Thumbs.db`, any `*.log`); (3) any `.env*` that is not a `.example` template, since real env files can contain secrets. The currently-tracked `tsconfig.tsbuildinfo` files are carved out (machine-generated but in the active WIP set). Pure-bash + awk, sub-second; use `bash scripts/check-tracked-artifacts.sh --list` to see rules and counts.

### Release-version sync check
```bash
pnpm version:check  # exit non-zero if the 6 release manifests disagree
```
A UFOP release ships six related artifacts: the desktop binary (`src-tauri/tauri.conf.json` + `src-tauri/Cargo.toml`), the CLI (`cli/Cargo.toml`), the root JS package (`package.json`), the admin console (`admin/package.json`), and the marketing site (`marketing/package.json`). `scripts/release-local.sh --version X.Y.Z` names artifacts after `--version` but **does not** auto-bump the six manifest files — that's a pre-release step done by hand. A forgetful partial bump otherwise produces `FileManager-1.0.0.dmg` that reports its internal version as `0.9.0`, breaks the Tauri updater's diff logic, and leaves admin/marketing showing a stale version next to the bumped desktop. The check parses each manifest (JSON files via top-level `"version":` key; TOML via `^version =` in `[package]`) and fails on any drift. Pre-release tags (`1.0.0-rc.1`) and build metadata (`1.0.0+build.42`) must match exactly across all six. Pure-bash + grep + sed, no jq; sub-second.

### Tauri version pinning check
```bash
pnpm tauri:versions  # exit non-zero if JS and Rust Tauri deps disagree on major
```
Tauri's JS layer (`@tauri-apps/api`, `@tauri-apps/cli`, `@tauri-apps/plugin-*`) and its Rust layer (`tauri`, `tauri-build`, `tauri-plugin-*`) share an IPC wire format that is stable **within** a major version but breaks across majors. Bumping `@tauri-apps/api` to `^3.0.0` while leaving `tauri = "2"` in Cargo.toml compiles cleanly on both sides, but every IPC call fails at runtime with cryptic "invalid invoke arguments" / type-mismatch errors. The check parses each Tauri-related dependency (root + admin + marketing `package.json` for JS, `src-tauri/Cargo.toml` for Rust — both inline `tauri = "2"` and object `tauri = { version = "2", features = [...] }` forms supported), normalizes range specs (`^`, `~`, `=`, wildcards, pre-release tags), and verifies all majors match. `workspace:*` / `catalog:*` / `link:*` / `file:*` / `git:*` refs are skipped (they're resolved by pnpm, not directly comparable). Pure-bash + grep + sed, no jq; sub-second.

### Bash script hygiene check
```bash
pnpm hygiene:check  # exit non-zero if any scripts/*.sh lacks shebang / set / exec bit
```
The `scripts/` directory is the project's verification + release surface — every check the verify gate runs lives here. The check enforces three minimum invariants per script: (1) first line is `#!/usr/bin/env bash` (portable shebang — `/bin/bash` ties the script to whichever bash version ships with the OS, which on macOS is 3.2); (2) a `set -uo pipefail` or `set -euo pipefail` directive appears at column 1 anywhere in the file (both forms are accepted — the `-e` is a per-script design choice; iter-9+ check scripts accumulate failures so they intentionally omit `-e`, older scripts fail fast); (3) the executable bit is set, so `./scripts/foo.sh` works in addition to `bash scripts/foo.sh`. Pure-bash, sub-second; use `bash scripts/check-bash-hygiene.sh --list` to see all three columns.

### CHANGELOG version sync check
```bash
pnpm changelog:check  # exit non-zero if CHANGELOG has no `## [package.json version]` section
```
Iter 17's `version` stage keeps the six release manifests in sync, but they're just numbers — they say *nothing* about what changed. This check closes the remaining release-time gap: shipping `FileManager-0.2.0.dmg` without a `## [0.2.0]` section in `CHANGELOG.md` means users see a binary version bump with zero release notes. The check uses Keep-a-Changelog conventions: section heading must be `## [X.Y.Z]` (bare semver, no `v` prefix); a `## [Unreleased]` section is allowed in addition (work in progress between releases) but does NOT substitute for the versioned entry; pre-release tags (`1.0.0-rc.1`) and build metadata (`1.0.0+build.42`) must match the manifest version exactly. Pure-bash + grep + sed, no jq; sub-second.

### GitHub Actions disabled check
```bash
pnpm gha:check  # exit non-zero if any active workflow appears in .github/workflows/
```
The project disables GitHub Actions entirely by policy (commit 50180c3, `.github/workflows/README.md`): CI/CD runs locally and publishes container images directly to GHCR from the developer's machine. The two former workflow files (`ci.yml`, `release.yml`) are renamed with `.disabled` suffixes so GitHub's workflow scanner ignores them. This check guards against accidental re-enablement: any `*.yml` or `*.yaml` directly inside `.github/workflows/` at the repo root is rejected with a clear `mv X.yml X.yml.disabled` hint. Nested workflows under `node_modules/`, the `unified-file-ops/` stale snapshot, and `.github/actions/*/action.yml` (composite actions, not workflows) are intentionally ignored — GitHub Actions only fires on root-level workflows. Also confirms `.github/workflows/README.md` is present so the contrarian path (re-enable) stays discoverable. Pure-bash, sub-second.

### pnpm audit (critical CVE) check
```bash
pnpm audit:check  # exit non-zero on any critical-severity CVE in --prod deps
```
This is the verify gate's only security-scanning stage. It wraps `pnpm audit --audit-level=high --prod` (production deps only — dev tooling like vitest/eslint/prettier never ships, so its vulnerabilities are out of scope). The threshold was raised from `critical` to `high` in iter 23 after Next.js was bumped 15.5.12 → 15.5.18 to close the 8 standing high-severity advisories. The remaining 1 moderate PostCSS finding (transitive through Next.js's own deps) is surfaced as info; bump the threshold to `moderate` once that's patched upstream. Network errors (unreachable npm registry) are tolerated as warnings, not gate-blocking, so offline development continues to work — the script distinguishes "found a critical CVE" from "couldn't fetch the advisory database" via the presence of a parseable severity summary. Pure-bash wrapper, ~1s with warm registry cache.

### Release-readiness config check
```bash
pnpm config:check  # exit non-zero if Tauri config has shippable defects
```
Validates `src-tauri/tauri.conf.json` + `src-tauri/capabilities/*.json` + `.env.example` for placeholder bundle identifiers (`com.tauri.dev`, `com.example.*`, etc.), placeholder productName/version, over-broad capabilities (`allow-all`, `dangerous`, `withGlobalTauri`), missing env template, and updater incoherence (active=true with empty `pubkey` or no endpoints). Pure-bash, no `jq` / python dep, runs in well under a second. Mirrors Section 5E of the engineering-audit skill.

### Git pre-push hook (opt-in)
```bash
pnpm hooks:install    # install .git/hooks/pre-push → runs verify:fast on every push
pnpm hooks:status     # show whether the hook is installed
pnpm hooks:uninstall  # remove the hook
```
Bundles the entire local CI gate into git itself. Once installed, every `git push` that carries commits runs `pnpm verify:fast` (≈25s, see above) and aborts the push on failure. Tag-only pushes and branch-deletes are skipped (no code to check). The installer refuses to overwrite a pre-existing pre-push hook unless `--force` is passed, and `--uninstall` only removes hooks it installed itself. Bypass with `git push --no-verify` only in genuine emergencies — the gate is the substitute for the (intentionally disabled) GitHub Actions pipeline.

## Architecture quick reference

Full architectural notes (IPC boundary, AppState pattern, command module wiring, error handling, traits, migrations, connector protocol pattern, three-layer transfer engine) live in **[CLAUDE.md](CLAUDE.md)**. That file is the authoritative source for adding new features end-to-end.

When adding a new feature, follow the **end-to-end checklist** in CLAUDE.md — it covers:
1. Rust engine module
2. Tauri command module + `invoke_handler` registration
3. `AppState` wiring (both init paths!)
4. SQLite migration + test assertions
5. Zustand store
6. React panel component
7. Toolbar entry
8. Command palette entry

## AI-agent workflow (for contributors using Claude Code / Cursor / similar)

This repo is set up to be driven by AI coding agents. The configuration lives in `ai-agents/`:

1. Read `docs/PRD-V5-Definitive.md` for full product spec
2. Read `ai-agents/README.md` for agent loading instructions
3. Load `ai-agents/global-policy.yaml` + a specific agent for any task

Agent governance rules are in `docs/AI-Agent-Governance-Addendum.md`.

## Releasing

This project **does not use GitHub Actions**. Releases are cut from a local Mac with both the Apple Developer signing identity and the Tauri updater key configured.

1. Fill in `.env.local` (copy `.env.example` first). Required: `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, plus the Apple `APPLE_*` vars.
2. Run `./scripts/release-local.sh --check` to verify prereqs.
3. Tag and publish: `./scripts/release-local.sh --version 1.0.0` (or `--prerelease` for RC tags).
4. For stable releases, manually open a PR to [`ClappeDiya/homebrew-tap`](https://github.com/ClappeDiya/homebrew-tap) updating `Casks/unified-file-ops.rb`.
5. Windows signing: pending SignPath Foundation approval; until then ship `.msi`/`.exe` unsigned with a download-page disclaimer.

The `.github/workflows/*.yml.disabled` files are reference scaffolding only — they document the CI flow we'd use if Actions were ever turned back on, but they are not executed.

## Versioning

- PRD: V5 (Definitive)
- Agent Governance: v1.0
- Agent Pack: v1.0
- Sprint Plan: 42 sprints, 8 phases, 21 months

## License & contribution policy

This project is source-available under [PolyForm Shield 1.0.0](LICENSE). By submitting a contribution you agree it is licensed under the same terms. The Shield license prohibits using the software to build a competing product but otherwise permits commercial use, modification, and redistribution.
