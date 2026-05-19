# Changelog

All notable changes to the Unified File Operations Platform are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Security

- Bumped Next.js 15.5.12 → 15.5.18 in `admin/` and `marketing/` to close 8 high-severity advisories: DoS via Server Components (GHSA-q4gf-8mx6-v5v3, GHSA-8h8q-6873-q5fj), Middleware/Proxy bypass in App Router and Pages Router (GHSA-36qx-fr4f-26g5 et al.). Verify-gate `audit` stage threshold raised from `critical` to `high` to lock in the protection.
- Verify-gate `audit` stage threshold raised from `high` to `moderate`. Added a structured GHSA suppression list to `scripts/check-pnpm-audit.sh` with one entry: `GHSA-qx2v-qp2m-jg93` (PostCSS XSS via Next.js's bundled-and-pinned `postcss@8.4.31`). The advisory is uncorrectable from this repo — Next 16.2.6 still pins the same vulnerable version — so the suppression carries a justification and a quarterly review date (`2026-08-19`). Each suppression follows the same convention as the iter 24/25 allowlists: justification + review window required.

### Tooling

- New verify-gate stage `ws-deps` (`scripts/check-workspace-deps.sh`, `pnpm ws-deps:check`) fails the gate when a shared dependency is **installed** at different MAJOR versions across workspace members. The check reads `pnpm-lock.yaml` to compare **resolved** versions rather than declared ranges; this suppresses the noise where two members declare different carets (`^19.0.0` vs `^19.2.4`) that resolve to the same installed version. Falls back to declared-range comparison only when no lockfile is present (fresh-clone usability). Two known-intentional major splits (`tailwindcss`, `tailwind-merge` — v3 in Next.js apps until Next 16 ships v4 support, v4 in the Tauri shell) are allowlisted in-script with a justification.
- New verify-gate stage `csp` (`scripts/check-tauri-csp.sh`, `pnpm csp:check`) parses `app.security.csp` from `src-tauri/tauri.conf.json` and fails the gate on regressions to the WebView Content-Security-Policy: missing `default-src`, `'unsafe-inline'` or `'unsafe-eval'` in `script-src` (directly or inherited via `default-src`), bare `*` wildcards in any directive, or `data:` scheme in `script-src`. `style-src 'unsafe-inline'` is accepted (industry convention for Tailwind/shadcn runtime style injection). Catches the textbook "added 'unsafe-inline' for one debug session and forgot to revert" XSS-defense regression.
- New verify-gate stage `stale-supp` (`scripts/check-stale-suppressions.sh`, `pnpm stale-supp:check`) parses every entry in the `GHSA_IGNORE` block of `scripts/check-pnpm-audit.sh` and fails the gate when any suppression's `review-by` date is strictly in the past, or when an entry is malformed (missing fields, non-GHSA id, non-ISO-8601 date). Closes the iter 26 audit finding: suppressions added "for now" no longer rot silently forever — the gate forces re-evaluation on each entry's scheduled date. Same `name|reason|YYYY-MM-DD` convention applies to any future allowlist that gains a review window.
- Extended `scripts/check-changelog.sh` (iter 21) to validate `## [Unreleased]` section content shape when present: at least one recognized subsection header is required, every subsection must contain at least one bullet entry, and subsection names must come from Keep-a-Changelog's vocabulary (`Added`, `Changed`, `Deprecated`, `Removed`, `Fixed`, `Security`) plus this project's `Tooling` extension. Catches the "bumped a dep, added the Unreleased heading, but never filled it in" drift pattern. The existing version-matching check is preserved unchanged.
- Extended `scripts/check-command-unwraps.sh` (iter 14) to also flag the panic-macro family inside `#[tauri::command]` bodies: `panic!(`, `unreachable!(`, `todo!(`, `unimplemented!(`. All have identical IPC blast radius to `.unwrap()`/`.expect(` (panic the host runtime → opaque frontend error → poisoned session). Word-boundary check prevents false positives on user identifiers like `mypanic!` or `panicked()`. `assert!()` / `debug_assert!()` are intentionally NOT flagged (idiomatic invariant checks).
- New verify-gate stage `caps` (`scripts/check-tauri-capabilities.sh`, `pnpm caps:check`) parses every `src-tauri/capabilities/*.json` and fails the gate when any sensitive permission (currently `fs:*`) ships unscoped — either as a string-form identifier (no `allow` block at all) or as an object with an empty `allow` array. Both forms grant filesystem access to the entire host. Iter 10's `check-tauri-config.sh` covers structural anti-patterns (`allow-all`, `withGlobalTauri`); this complements it with scope coverage on the path-scoped permissions. All 8 current `fs:allow-*` permissions are correctly scoped; this stage locks in regression-prevention.
- New verify-gate stage `cargo-audit` (`scripts/check-cargo-audit.sh`, `pnpm cargo-audit:check`) runs `cargo audit --json` against the Rust dependency tree, symmetric to iter 22/26's `pnpm audit`. Uses the same `id|reason|review-by-YYYY-MM-DD` suppression list convention (`RUSTSEC_IGNORE`). Auto-skipped offline-style when `cargo-audit` is not installed. Three standing suppressions: `RUSTSEC-2026-0098 / 0099 / 0104` — all in `rustls-webpki 0.103.10`, reached only through `tauri 2.10.3 → reqwest → rustls-platform-verifier`. Uncorrectable from this repo's Cargo.toml; review 2026-07-19 and bump Tauri if a patched release exists. 22 unmaintained/unsound/yanked warnings are reported informationally but never fail the gate.
- Extended iter 27's `scripts/check-stale-suppressions.sh` to ALSO walk the new `RUSTSEC_IGNORE` block in `scripts/check-cargo-audit.sh`. Generic `SOURCES` table now drives parsing — each source declares its file path, heredoc variable name, and id-regex. Heredoc-tag-drift safety net per source. Reports across both audit sources in one summary; 4 active suppressions total (1 GHSA + 3 RUSTSEC), all CURRENT.
- New verify-gate stage `ret-types` (`scripts/check-command-return-types.sh`, `pnpm ret-types:check`) enforces CLAUDE.md's invariant *"All async Rust errors return `Result<T, AppError>`"*. Each `#[tauri::command]` must return either `Result<T, AppError>` (canonical) or a bare `T` (infallible). Surfaces `Result<T, ()>`, `Result<T, String>`, `Result<T, Box<dyn Error>>` etc. — they compile cleanly but degrade the frontend's structured error rendering. Awk scanner buffers lines from `#[tauri::command]` to the next `{`, parses fn name + return type. Sub-second on ~350 commands.
- New verify-gate stage `env-example` (`scripts/check-env-example-drift.sh`, `pnpm env-example:check`) fails the gate when a user-facing env var is referenced in code but not declared in any `.env.example` template. Catches the onboarding-breaking drift pattern: dev adds `process.env.NEW_VAR` (or `env::var("NEW_VAR")`) to code but forgets to add it to `.env.example`, leaving a fresh clone with a silently-undefined feature flag. Scans `process.env.*`, `import.meta.env.*`, `env::var("…")`, and `env!("…")`. Skips OS-builtins (`HOME`, `PATH`, `CARGO_PKG_VERSION`, `KRB*`, `AWS_*` runtime config, etc.). Vars declared in any `.env.example` but unreferenced by code are reported as informational only (intentional pre-declared placeholders for cloud connectors / future features).
- Extended `scripts/check-migrations.sh` (iter 11) with **content-discipline checks**. Each `Migration { version, description, sql }` entry in `all_migrations()` is now validated: non-empty `description`; `sql:` field must reference a constant named exactly `V<version>_SCHEMA` (codifies the naming convention so a copy-paste bug like `version: 12, sql: V11_SCHEMA` is caught); the referenced constant must exist; its body must contain at least one DDL/DML/PRAGMA keyword (empty stubs silently no-op at runtime → schema drift); each `V*_SCHEMA` constant must be referenced exactly twice in the file (definition + one use — orphan constants and accidental re-use both fail). All 11 current migrations pass; this locks in regression-prevention for the schema-evolution surface that CLAUDE.md flagged as a footgun.

### Fixed

- `archive_create` IPC command (`src-tauri/src/commands/archive_commands.rs`) was using `unreachable!()` for the impossible format-mismatch arm of its format-dispatch `match`. The earlier allow-list validation guarantees the arm is dead under correct logic, but if the two lists ever drifted apart the IPC handler would panic instead of returning a structured error. Replaced with a defensive `Err(...)` of the same `Box<dyn std::error::Error>` type. Discovered by the iter 29 panic-macro extension to `scripts/check-command-unwraps.sh`.
- `safety_assess_intent` and `safety_confirm_intent` IPC commands (`src-tauri/src/commands/safety_commands.rs`) were declared `Result<T, ()>` — opaque error type that gives the frontend nothing to render. Tauri requires async commands with `State<'_, _>` references to return `Result`, so a bare-T return is not available; changed to canonical `Result<T, AppError>` and updated docs. Bodies remain fail-open (`Ok(...)`); the new shape leaves room for future structured error paths. Discovered by the iter 30 `ret-types` check.
- 6 user-facing env vars were referenced in code but missing from every `.env.example` template — fresh clones had no signpost for them: `ADMIN_DB_PATH` and `ADMIN_EMAIL`/`ADMIN_PASSWORD` (bootstrap-admin seeding in `admin/src/app/api/auth/login/route.ts`), `NEXT_PUBLIC_APP_NAME` and `NEXT_PUBLIC_API_URL` (admin client-bundle config), `TAURI_DEV_HOST` (Vite/Tauri dev-server LAN bind). Added entries to `admin/.env.example` and root `.env.example` with documentation comments. Discovered by the iter 31 `env-example` check.

### Changed

- Aligned `admin/package.json` and `marketing/package.json` declared ranges with the root workspace where the lockfile already resolves to the same version: `react`, `react-dom` (^19.0.0 → ^19.2.4), `@types/react` (^19.0.0 → ^19.2.14), `@types/react-dom` (^19.0.0 → ^19.2.3), `autoprefixer` (^10.4.20 → ^10.4.27), `postcss` (^8.4.47 → ^8.5.8), `typescript` (^5.6.3 → ^5.9.3). Documents the supported floor and stops declared/resolved drift from re-emerging if a developer re-installs from scratch with `--prefer-lowest`.
- Aligned `lucide-react` across all workspace members: `admin/` and `marketing/` bumped ^0.460.0 → ^0.577.0 to match root. Lucide v0.460 → v0.577 includes one breaking surface — six `*Circle`/`*Triangle` icons were renamed (`AlertCircle` → `CircleAlert`, `CheckCircle` → `CircleCheckBig`, `CheckCircle2` → `CircleCheck`, `HelpCircle` → `CircleQuestionMark`, `XCircle` → `CircleX`, `AlertTriangle` → `TriangleAlert`) — but v0.577 still re-exports every old name as a deprecated alias, so the 59 icons used across `admin/src` and `marketing/src` continue to work without source changes.

## [0.1.0] - 2026-03-15

### Added

#### Desktop Application
- Tauri 2.0-based cross-platform desktop app (macOS 10.15+, Windows, Linux)
- Dual-pane file manager with tabbed navigation and four view modes (list, detail, grid, compact)
- Simple mode (default) with streamlined sidebar: Files, Transfers, Sync, Cloud & Servers, Search, Favorites, Activity, Settings
- Advanced mode with full feature set: dual panes, command palette, filter bar, data grid, terminal panel
- Instant mode switching between Simple and Advanced without restart
- Onboarding wizard with 6 steps: Welcome, Choose Style, Connect Locations, Quick Start, Compatibility, Ready
- 17 guided flow wizards for common tasks (cloud connections, transfers, sync, backup, migration)
- Activity feed with structured error reporting (what happened, why, what the app did, what user can do)
- Command palette (Ctrl/Cmd+K) for quick access to all features
- Context menu with full file operations
- Virtual scrolling for large directories via @tanstack/react-virtual

#### File Operations
- Copy, move, rename, duplicate, delete, create folder, create file
- Undo support (up to 10 levels) for all file operations
- File metadata inspection
- Breadcrumb navigation with path segments

#### Transfer Engine
- Multi-protocol file transfers with queue management
- Pause, resume, cancel, retry operations
- Priority levels (high, normal, low) with queue reordering
- Bandwidth throttling (global and per-connection)
- Conflict resolution policies (ask, overwrite, skip, rename, newer-wins)
- Three-tier verification: Quick (size), Standard (xxHash3), Full (SHA-256)
- Transfer history with search, export (CSV), and cleanup
- Three-layer journal for crash recovery
- Automatic recovery of interrupted transfers on restart

#### Connectors
- SFTP: SSH-based transfer with key auth, jump hosts, host key verification
- FTP/FTPS: Active/passive mode with TLS support
- WebDAV: HTTP-based DAV with Nextcloud/ownCloud compatibility
- SMB/CIFS: Windows shares and NAS devices with LAN discovery
- NFS: NFSv3/v4 with Unix permission display
- Local Drive: Drive-to-drive transfers with preflight checks
- Amazon S3: S3 and S3-compatible services (MinIO, Wasabi, B2 S3)
- Google Drive: OAuth 2.0, My Drive/Shared Drives, Docs export, versioning
- Dropbox: OAuth 2.0, team/shared folders, session uploads
- OneDrive: Personal/Business, SharePoint, naming restrictions
- Backblaze B2: Native API with large file uploads

#### Connection Management
- Save, test, and organize connections into groups
- Import/export connection profiles (JSON)
- OS keychain credential storage (macOS Keychain, Windows Credential Manager, Linux Secret Service)

#### Sync Engine
- Bidirectional and one-way sync modes
- Manual, watcher (filesystem events), and scheduled (cron) triggers
- Dry run with preview export (CSV)
- Conflict detection and resolution (keep source, keep dest, keep both, skip)
- Quarantine for conflicting files
- Rollback to pre-sync state
- Health indicators (green, yellow, red, gray)
- Sync reports with CSV and JSON export

#### Peer-to-Peer
- mDNS/Bonjour peer discovery on local network
- Trust levels: untrusted, trusted, blocked
- TLS-encrypted peer transfers
- Manual connection via IP address
- Transfer request and approval flow

#### Server-to-Server Transfers
- Direct transfer between remote endpoints
- Capability matrix for transfer method selection
- Pause, cancel, retry support

#### AI Assistant
- Chat interface with contextual assistance
- Error explanation (plain language + causes + fixes)
- Proactive suggestions based on user context
- Natural language job creation ("Sync my Documents to Drive every night")
- Safety controls: confirmation gate for destructive actions
- Full audit logging of all AI interactions
- Feature toggles for granular control
- Local/cloud model routing

#### Terminal
- Built-in local terminal with default shell detection
- Remote SSH terminal via saved connections
- Multiple sessions with tab switching
- Split layouts (horizontal/vertical)
- Path escaping for drag-and-drop

#### Encryption & Security
- Encrypted vaults with AES-256-GCM and ChaCha20-Poly1305
- Argon2id key derivation with zeroize memory safety
- Encrypt-for-upload / decrypt-from-download
- Vault password management
- Transport security checking and HTTPS enforcement
- Encryption policies (per file type, per destination)

#### Cross-Platform Compatibility
- Compatibility engine for file name validation across platforms
- Detection of Windows reserved names, invalid characters, path length limits
- Unicode normalization (NFC/NFD) handling
- Safe auto-renaming with undo capability
- Compatibility badges on files

#### Batch Rename
- Pattern-based renaming with tokens ({name}, {ext}, {num}, {date}, {parent}, {counter})
- Find/replace (literal and regex)
- Case transforms (upper, lower, title, sentence)
- Preview before apply
- Batch undo

#### Preview Engine
- File preview pane with EXIF metadata for images
- Text file preview
- Multiple file format support

#### Archive Tools
- Browse, create, extract archives
- ZIP, TAR, TAR.GZ, 7Z format support
- Password-protected archives
- Configurable compression levels

#### Integrity Tools
- Checksum computation and verification (MD5, SHA-1, SHA-256)
- Duplicate file finder with resolution actions
- Tags and color labels
- Smart folders with dynamic rules

#### Themes & Accessibility
- Four themes: System (auto), Light, Dark, High Contrast
- Instant theme switching via CSS custom properties
- Full keyboard navigation with visible focus indicators
- ARIA labels and screen reader support
- Design token system for consistent styling

#### Admin Console
- Next.js web application for organizational administration
- Dashboard with stats cards and overview panels
- User management with role-based access (Super Admin, Org Admin, Manager, User, Viewer)
- Device fleet management with policy compliance tracking
- Policy engine with enforce/warn/audit modes
- Approval workflows for sensitive operations
- Tamper-evident audit log with cryptographic chaining
- Connector management (organization-wide)
- Workspace management
- AI governance settings and audit
- Billing page
- OpenAPI 3.0 specification at /api/openapi
- REST API for all admin operations
- Webhook subscriptions for event notifications

#### CLI
- Rust-based CLI (`ufop`) with human/JSON/YAML output formats
- Commands: login, connection (list/add/test/remove), transfer, sync (list/create/run/delete), compat, checksum, duplicates, rename, archive (create/extract/list), status
- Dry-run mode for safe previewing
- Bandwidth limiting
- Configurable verbosity levels
- Exit codes for scripting

#### Design System
- Shared design tokens package (colors, typography, spacing, shadows, radii, animations)
- Shared UI components package (button, dialog, dropdown, input, badge, tabs, tooltip, scroll area, separator)

#### CI/CD
- GitHub Actions CI pipeline with lint, type check, test, format verification
- Supports both frontend (TypeScript/React) and backend (Rust) validation
- Concurrency groups to cancel superseded CI runs

### Security
- OS keychain integration for credential storage
- TLS enforcement for all supported protocols
- CSP configuration for the WebView
- Tauri plugin-based file system access
- Audit logging for all operations
- AI action confirmation gate for destructive operations
