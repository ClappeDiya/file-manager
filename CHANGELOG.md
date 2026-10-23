# Changelog

All notable changes to the Unified File Operations Platform are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Interface polish:
  - **Sortable, paged backup tables**: the version list, a version's files and the search results in Restore, the run history and the destination files in Maintenance sort by a header click (keyboard too, with `aria-sort`) and page with 25/50/100/250/All rows (All is virtualized); page size and sort are remembered per table. The generic `DataGrid` gained columns, paging, server-side mode and remembered views for this. Files and run history are paged by the backend: `backup_list_files`, `backup_run_history` and `backup_remote_list` take an optional `page` `{offset, limit, sort, descending, filter}` and answer `{items, total, offset, limit}` (plain arrays without it); REST `GET /api/v1/jobs/{id}/files` and `GET /api/v1/runs` take `offset`, `limit`, `sort`, `desc`, `filter`. Paged file lists have no 20 000-row cap.
  - **Text size** (Settings → Appearance): Small, Default, Large or Larger, applied to the root font size so the rem-based interface scales; Ctrl/Cmd + `+` / `-` / `0`, also in the command palette and the shortcut cheat sheet. Persisted (UI state version 2; invalid stored values fall back to the default).
- CLI, tooling and packaging:
  - **`ufop backup bench`**: compression (stored, deflate, zstd levels, bzip2), AES-256-GCM vs ChaCha20-Poly1305, Argon2id presets with suggested parameters for a target time, Fixed vs FastCDC chunkers (speed, size distribution, deduplication of an edited copy) and SHA-256 vs SHA-512 on generated (`--seed`) or your own sample data, through the engine's real code paths; `--repeat`, `--threads`, `--size`, `--only`, `--json` and `--print-options` (the recommended `ufop backup run` flags). Also Settings → Benchmark in the desktop app (Advanced) and `backup_bench`.
  - **`ufop backup debug`**: read-only `header`, `volumes` (kind, size, shard; filters), `stats` (per kind, blocks, compression and dedup ratios, per-version referenced and unique bytes), `blocks`, `block <hash>` (`--raw`, `--out`), `index`, `fileset`, `sessions` (lock holders) and `db`. The header and statistics are also in Maintenance → Inspect (Advanced) as the `inspect_header` / `inspect_stats` operations.
  - **Shell completions and man pages**: `ufop completions bash|zsh|fish|powershell|elvish` and `ufop man [--out DIR] [command…]`, generated from the command tree (`ufop_core::cli_gen`); the same for `ufop-server` (man section 8). Installed by the new CLI packages.
  - **Release tooling** (local scripts, no CI): `scripts/release-local.sh` builds every target the machine can build and says what it skipped and why (`--print-plan`, `--dry-run`, `--no-publish`, `--target`, `--windows`/`--no-windows`); Windows is a first-class target (MSI + NSIS on Windows, NSIS from Linux/macOS with `--windows-cross xwin`), signed through Tauri's `signCommand` with signtool (`packaging/windows/sign.ps1`) or osslsigncode (`packaging/windows/sign.sh`) when a certificate is configured, and winget manifests are generated (`packaging/windows/winget/generate-manifest.sh`). CLI packages with completions and man pages: `.tar.gz`/`.zip`, a `.deb` on Linux (`packaging/linux/package-cli.sh`) and a Homebrew formula on macOS (`packaging/macos/generate-formula.sh`).
  - **Signed checksums**: every release has `SHA256SUMS` over all assets, signed with the updater's minisign key (`SHA256SUMS.minisig`) and optionally GPG (`--gpg-key`, `SHA256SUMS.asc`), verified before upload (`scripts/release-checksums.sh`). Users check downloads with `scripts/verify-release.sh` (minisign, the ufop CLI or OpenSSL 3) or `ufop verify-release SHA256SUMS`.
  - **Portable mode** for the desktop app (see `docs/user-guide.md`): `UFOP_PORTABLE_DIR`, `ufop-portable.json` or a `portable` file next to the executable keeps all app data (databases, backup job databases, logs, caches including the volume cache, previews, temporary files, webview data on Windows/Linux) in one folder, resolved by a single `paths` module; credentials go to an encrypted vault file in that folder (key file, or Argon2id from `UFOP_PORTABLE_PASSPHRASE`) instead of the OS keychain. Settings shows *Portable mode: on (path)* and the folders (`get_portable_status`).
  - **ufop-server web UI branding** (see `docs/server-operations.md`): title prefix, product name, logo, accent color and sanitized custom CSS from `--branding-file`/`--brand-*` flags, `PUT /api/v1/server/branding` (admin) or the console's managed `serverBranding` (highest, locks local changes); the web UI reads the public `GET /api/v1/branding`, the logo and stylesheet are served at `/branding/logo` and `/branding/custom.css`, and `index.html` is served already branded. A custom web UI folder stays the `--webroot` flag.
  - **Multi-arch server image**: `docker/ufop-server.Dockerfile` builds for `linux/amd64` and `linux/arm64` (`TARGETARCH`, web UI built once on the build platform, per-arch cargo caches), and `scripts/docker-build.sh` publishes one manifest list to GHCR (`--platforms`, `--push`, `--tag`, `--append-node`, `--dry-run`).
- Backup notifications (see `docs/backup-notifications.md`):
  - **Notification profiles**: reusable channels kept once (Backup settings → Notification profiles: list, add, test; minimum severity and templates in Advanced mode), chosen per job (`notification_profile_ids`) or by policy templates; a job sends to its own channels plus its profiles, the same channel once. Deleting a profile in use is refused unless forced, which removes the references with warnings. Organization profiles arrive read-only from the console's managed app settings (`notificationProfiles`), with secrets only as console credentials. IPC `backup_*_notification_profile*`, REST `/api/v1/notification-profiles`, `ufop backup notify-profile list|show|create|update|delete|test`.
  - **Webhook channel** with method (`POST`, `PUT`, `PATCH`, `GET`), content type, custom headers (secret values in the secure store, redacted from exports and bug reports; header names validated, no CR/LF injection) and body templates whose placeholders are escaped for JSON, form or HTML bodies (`{{json var}}`, `{{raw var}}`). **Presets** for Pushover, ntfy, Discord, PagerDuty Events v2 (trigger on problems, resolve on the next success, one dedup key per job) and Gotify, built by the engine (`backup_webhook_preset`, `POST /api/v1/notifications/webhook-preset`). HTTP reports take headers and `PATCH` too. Slack and Teams channels are in the job editor.
  - **HTML email**: `multipart/alternative` messages (text and HTML, base64, RFC 2047 subjects split into short encoded words) with a default report table or an HTML template with auto-escaped values; on for new email channels, off for existing ones. Preview in a sandboxed frame (`backup_preview_notification`, `POST /api/v1/notifications/preview`).
- ufop-server operations and security (see `docs/server-operations.md`):
  - **Graceful shutdown** on SIGTERM/SIGINT, Windows service stop and `POST /api/v1/server/shutdown {grace_secs}` (`ufop server shutdown`): new state-changing requests get `503` with `Retry-After`, running backups stop at a safe point and write a partial version within the grace period (`--shutdown-grace`, default 60 s), then the server exits 0; a second request stops at once. The desktop app drains running backups the same way when it quits.
  - **CSRF protection** for cookie-authenticated requests: a script-readable `ufop_csrf` cookie (an HMAC of the refresh token) must come back as `X-CSRF-Token` on state-changing requests that carry the refresh cookie (`/auth/refresh`, `/auth/logout`); bearer-token requests are exempt; cross-site `Origin`/`Referer` is refused. The web UI and `ufop server` send it (sign in once more after upgrading).
  - **Request tracing**: `X-Request-Id` (a safe client value or a new UUIDv4) echoed in responses, added to JSON error bodies and carried by a tower-http `TraceLayer` span with method, route template, status and latency; one log line per request (`--request-log on|off`), with query secrets and credential headers redacted.
  - **Prometheus metrics** at `GET /api/v1/metrics` (read scope) and optionally on `--metrics-listen` (with `--metrics-basic-auth`): build info, HTTP requests and latency by route template and status, backup runs by job and result, last success time, duration, bytes and files per job, running and queued operations, repository locks, next scheduled runs and the volume cache.
  - **OpenTelemetry trace export** (OTLP/HTTP JSON, `--otlp-endpoint`, `--otlp-headers`, `--otlp-service-name`, `--otlp-sample-ratio`): request spans and backup operation spans (job id, operation, result) with W3C `traceparent` propagation, batched through a bounded queue and flushed on shutdown. **Prometheus push gateway** (`--pushgateway-url`, job, instance, interval, basic auth or bearer) with a final push on shutdown.
  - **API tokens limited to jobs** (`job_ids`, `ufop server tokens create --job <id|name>`): job routes, job lists, run-all, tasks and history stop at the token's jobs, and server-wide routes need the admin scope as well.
  - **Server accounts and per-job access rules**: accounts with Argon2 passwords (or provisioned hashes), roles admin/operator/viewer, per-job rules by user, role or everyone with job globs (strongest match wins), enforced on every REST route, operation and RPC command; tokens never exceed their owner. The original server password becomes the `admin` account. Web UI *Users & access* with a preview before deleting a rule, `/api/v1/users` and `/api/v1/acl`, `ufop server user|acl|whoami`, `ufop-server user …` and `ufop-server hash-password`, and accounts provisioned read-only by the management console (`serverAccess` in managed app settings).
- **Ransomware hardening** (see `docs/backup-ransomware-hardening.md`): a guide to least-privilege credentials per provider (S3 policy JSON, B2 key capabilities, Azure SAS and roles, GCS roles), the two-credential model for compaction and retention, object lock and immutability, append-only SFTP, versioning and lifecycle, offline and second-provider copies; and a per-job checklist in the job editor (*Protection: 4/7* badge; in Advanced mode the checks with guide links and **Run checks**, which tries deleting a probe object with the backup credentials only when asked, and cleans it up). Also `ufop backup hardening <job> [--probe]`, `GET /api/v1/jobs/{id}/hardening`, `POST /api/v1/jobs/{id}/hardening/probe` and `backup_hardening_report`.

- Backup storage backends (see `docs/backup-destinations.md`):
  - **S3 credential chain**: static keys (default), environment, AWS profiles (static keys, `role_arn` with `source_profile` or `credential_source`, web identity), web identity token files, EC2 instance metadata (IMDSv2), ECS/EKS container credentials and *Automatic* (the SDK order). Temporary credentials refresh five minutes before they expire.
  - **STS AssumeRole** on top of any S3 credential source, with external id, session name, duration, STS endpoint and region; cached and renewed.
  - **Azure sign-in with Microsoft Entra ID**: service principal with a client secret or a certificate (PEM or PFX), workload identity federation (token file), and managed identity (App Service endpoint or instance metadata), plus explicit SAS. The Key Vault secret provider uses the same token code.
  - **GCS read-only scope** for read-only jobs and point-in-time views, and an explicit OAuth scope option.
  - **rclone process control**: call and startup timeouts, process groups killed with everything rclone started (also when cancelled), extra or isolated environment variables, and rclone's stderr in error advice.
  - **Storage class per volume kind** (data blocks, indexes, file lists, other) for S3, GCS and Azure access tiers. Tiers that need a thaw before reading (S3 Glacier/Deep Archive, Azure Archive) are refused here.
  - **Point-in-time reads of versioned S3 buckets**: "Open backup as it was at …" creates a read-only job that reads every object as it was at a time, for recovery after ransomware (`backup_open_point_in_time`, `POST /jobs/{id}/point-in-time`, `ufop backup open-at --point-in-time`).
  - **Request limits** per destination (requests per second for list, get, put and delete, and requests in flight) and **parallel listing** of shard folders, in the job editor and as `ufop backup` flags.
  - **Sharded layouts on cloud destinations**: S3, Azure, GCS, B2 and Swift prefixes, and SFTP, FTP, WebDAV and S3 connections.
  - **Live-provider contract tests** gated by `UFOP_LIVE_*` variables (`scripts/live-providers.sh`, and a nightly workflow shipped disabled per the repository's Actions policy).

- Backup policies, schedules and maintenance (see `docs/backup-policies-and-schedules.md`):
  - **Policy templates** jobs are based on, in order. Settings resolve as built-in defaults < global defaults < templates < the job's own settings < enforced organization templates, and older jobs keep their behaviour. Manage them in *Settings → Policy templates*, with `ufop backup template …`, `/api/v1/backup-templates` and import/export. Organization templates come through the console's managed app settings.
  - An **effective-settings view** in the job editor shows where each setting comes from, with *Reset to inherited* (`ufop backup effective`, `GET /jobs/{id}/effective-settings`).
  - **Cron schedules** for backups (5, 6 or 7 fields, several expressions, local time) with a preview of the next five runs (`ufop backup schedule-preview`, `POST /backup-schedule/preview`).
  - **Maintenance schedules**: compaction, verification and retention can each run on their own schedule. They take the right locks, show in the history and pause with the scheduler.
  - **Verification** can be limited to versions and paths, sample a number or a percentage of volumes, rebuild and hash-check a percentage of files, run downloads in parallel and stop after a number of errors. Problems name the affected versions and paths (*Custom check*, `ufop backup verify --version/--path/--percent/--files-percent/--parallel/--max-errors/--structure`).
  - **Async hook scripts** that start without being waited for (`before_mode`/`after_mode: async`).

- Restoring and reading backups (see `docs/backup-restore.md`): restore as it was at a date and time ("Restore as it was at …", `--time`, `--version latest|oldest`, `?time=` and `/versions/resolve`), restore into a ZIP, TAR or TAR.GZ archive streamed without staging ("Download as ZIP…", `--archive`), mirror restores that delete what the version lacks inside the restored folders after a dry-run list, the Safety Interlock and a single-use confirmation token (`--mirror [--yes]`, `confirm_delete`), file owners recorded at backup time and restored when running with privileges (`restore_owner` auto/always/never), a restore error policy per kind with fail-fast and an error budget, sparse-file restores that leave holes for zero runs, previewing one backed-up file in the preview pane and reading or exporting it (`ufop backup cat`, `GET /jobs/{id}/file` with ranges, `backup_read_file`), a bounded local cache of sealed volumes with LRU eviction, checked entries, stats and Clear cache (`ufop backup cache`, `/backup-cache`), and "Open as drive": a version or every version as a read-only, token-protected WebDAV drive on 127.0.0.1 mounted with gio, mount_webdav or net use (`ufop backup mount`).
- Backup sources and runs (see `docs/backup-sources-and-runs.md`): `.ufopignore` files and optional `.gitignore`/`.git/info/exclude` with full gitignore semantics (anchoring, `**`, negation, folder-only patterns, escapes, deeper files override), "Stay on one filesystem", a read-error policy per kind (files, folders, sockets/devices) with fail-fast and an error budget that stop with a partial version, percentage rehash that re-checks unchanged files and reports "content changed without timestamp change", backing up a command's output (`stdin_command`, e.g. `pg_dump`) or standard input (`ufop backup stdin`), an upload limit per run that continues on the next run, parallel hashing of large files with identical blocks, ZFS and btrfs snapshots (chosen automatically by filesystem), a pre-backup estimate (new data at most/at least and duration from recent runs; *Estimate* button, `ufop backup estimate`, `GET /jobs/{id}/estimate`), and *Back up all now* (job list, tray, `POST /jobs/run-all`, `ufop server run-all`).
- Backup versions (see `docs/backup-versions.md`): notes and `key=value` tags on versions (set per job or with `--note`/`--tag`, editable later, filter by tag), "Keep forever" pins that retention and purge never remove and delete only with `force`, the storage each version added (and files added/changed/deleted) in the version list, CLI and API, the computer and user that wrote each version (overridable to continue another computer's history), a retention preview (kept/deleted per version with the rule that keeps it) live in the job editor and through `ufop backup retention preview`/`POST /jobs/{id}/retention/preview`, count-based GFS retention (latest, hourly, daily, ISO-weekly, monthly, yearly; "Keep daily, weekly, monthly and yearly versions" preset), and copying chosen versions to another backup (only missing data; re-chunked across formats; `ufop backup copy-versions`).
- Shared backup repositories (format 2, see `docs/backup-repository-format.md`): several computers or jobs back up into one destination, identical data is uploaded once, each keeps its own versions and retention, compaction keeps every source's data and waits a safety delay for young volumes; share an existing repository from Maintenance or `ufop backup share`, list sources with `ufop backup sources`, browse another source in Restore.

- Backup repository safety (see `docs/backup-safety.md`): repository lock leases (`<prefix>-lock.json` for writers, one file per reader), renewed in the background, stale locks taken over with a warning, "Repository in use by …" in Maintenance and a confirmed Break lock (`ufop backup lock status|break`, `GET`/`DELETE /api/v1/jobs/{id}/lock`); a clock check against the destination's file times (warn at 5 minutes, stop at 1 hour); periodic checkpoints that keep a long backup restorable if it stops half way; read-only jobs and connections (`--read-only`) that only restore and browse; and an encrypted local database for encrypted backups, with its key in the OS keychain, one-click migration, `--db-key-source` in the CLI and automatic rebuilding when the key is lost.

- Backup repository format 2 (default for new backups; format 1 stays readable and writable): content-defined chunking (FastCDC), Zstandard blocks and compressed file lists, an envelope key so passwords change without rewriting data, ChaCha20-Poly1305, and an optional sharded folder layout. Upgrade in place with rollback until the first format 2 write, change the password, and reshard from Maintenance, the REST API or `ufop backup upgrade | change-password | reshard | format-info`. See `docs/backup-repository-format.md`.

- Robocopy feature parity for sync pairs: selection (`/S /E /LEV /XD /MAXAGE /MINAGE /XA /XJ /SL`), comparison (`/XO /XN /XC /XL /IS /FFT /DST`), cleanup (`/PURGE /XX /MOV /MOVE /CREATE`), copy engine (`/COPY:T /DCOPY:T /Z /IPG /R /W /MT /RH`), `/LOG[+]`, Robocopy exit codes, and "paste a Robocopy command" import with one-click presets.
- Windows-only Robocopy switches: `/COPY:S /COPY:O /COPY:U` (`/SEC`, `/COPYALL`), `/A /M /IA /XA:<letters> /A+ /A-`, plus `/TIMFIX` and `/SECFIX` (all platforms for timestamps).
- `ufop copy <src> <dst> [files] [/switches]` in the CLI — the same engine as the desktop app, Robocopy exit codes, `/L` list-only, `/MON` and `/MOT` monitoring, Ctrl+C stops after the current file.
- Live sync progress (bar, current file, time left) with a Cancel button; one run per pair at a time.
- Side-by-side Source | Destination preview before a sync, and a required confirmation when a run will delete files.
- `list_config` command so the settings viewer shows stored configuration.

### Fixed

- Mirror runs deleted the rollback snapshots they had just taken (stored inside the destination); snapshots and quarantine now live in the app data folder, and `.sync-rollback`, `.sync-quarantine` and `.ufop-partial` are never synced or purged.
- "Rollback", the quarantine list and "Ask" conflicts never had data: the executor's snapshots, quarantined files and pending conflicts are now recorded, and resolving a conflict applies the choice to the file.
- Copies didn't keep file dates, so every sync re-copied everything; conflict resolution now copies with dates too.
- Windows: transfers failed at the durability step ("Access denied" from a read-only `sync_all`); drive detection used the removed `wmic` and found no drives; SMB share listing sent credentials to `net view`, which doesn't accept them; the file watcher aborted on a misaligned pointer in `notify` 6 (now 8).
- Linux build broke on `smb_path`; `narrator` doctest failed to compile.
- 15 features whose frontend call didn't match the backend's arguments and failed every time: rename, SSH config lookup, third-party connection import, transfer preflight, auto re-upload of edited remote files, S3 lifecycle rules (which also crashed the view) and object versions, transfer-history search (results were never shown) and cleanup-by-age, duplicate resolution (was a no-op), CloudFront calls, settings viewer.
- CLI: `ufop checksum` printed made-up hashes for MD5, SHA-1 and SHA-256; the CLI didn't compile.

### Security

- Closed every open advisory: Next.js 15.5.26 (admin, marketing; incl. two critical RCEs), `react-router` 7.18, `uuid` 11, `postcss` via a `next>postcss` override, `nanoid`, `browserslist`, `sharp`, `@babel/core`; Rust `suppaftp` 10 (FTP CRLF command injection), `rustls` 0.23.45, `rustls-webpki` 0.103.15, `h2` 0.4.19, `quick-xml` 0.41. Both suppression lists are now empty.

- Bumped Next.js 15.5.12 → 15.5.18 in `admin/` and `marketing/` to close 8 high-severity advisories: DoS via Server Components (GHSA-q4gf-8mx6-v5v3, GHSA-8h8q-6873-q5fj), Middleware/Proxy bypass in App Router and Pages Router (GHSA-36qx-fr4f-26g5 et al.). Verify-gate `audit` stage threshold raised from `critical` to `high` to lock in the protection.
- Verify-gate `audit` stage threshold raised from `high` to `moderate`. Added a structured GHSA suppression list to `scripts/check-pnpm-audit.sh` with one entry: `GHSA-qx2v-qp2m-jg93` (PostCSS XSS via Next.js's bundled-and-pinned `postcss@8.4.31`). The advisory is uncorrectable from this repo — Next 16.2.6 still pins the same vulnerable version — so the suppression carries a justification and a quarterly review date (`2026-08-19`). Each suppression follows the same convention as the iter 24/25 allowlists: justification + review window required.

### Tooling

- New verify stages: `ipc-args` (every `tauriInvoke` call's argument names match its Rust command — found the 15 broken calls above), `clippy` (`-D warnings`), `core` and `cli` (shared engine and CLI: clippy + tests), `win-check` (clippy against `x86_64-pc-windows-gnu`, skipped without the toolchain), and `cargo-test` now runs integration and doc tests too.
- The sync engine and `AppError` moved to a new `crates/ufop-core` crate shared by the desktop app and the CLI.

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
- Extended `scripts/check-migrations.sh` again with a **destructive-DDL safety check**. Any `DROP TABLE` / `DROP INDEX` / `DROP VIEW` / `DROP TRIGGER` / `ALTER TABLE … DROP COLUMN` / `ALTER TABLE … RENAME (COLUMN|TO)` statement inside a `V<N>_SCHEMA` body must be preceded — within the prior 5 non-empty lines of the same body — by a comment starting with `-- SAFETY:` that justifies why the statement is safe and how user data is preserved. Default-deny: unjustified destructive DDL fails the gate. Data loss via an unconsidered schema migration is the single most catastrophic failure mode for the storage layer; this forces a written justification at write-time. All 11 current migrations are additive (zero destructive statements) so the check passes today — pure regression-prevention.
- New verify-gate stage `invoke-types` (`scripts/check-tauri-invoke-types.sh`, `pnpm invoke-types:check`) fails the gate when a `tauriInvoke(…)` (or `tauriInvokeSafe(…)`) call's return is **consumed** (assigned, returned, used in expression / arg / ternary) without an explicit `<T>` type argument. Statement-form `await tauriInvoke("delete_X", { … })` calls (return discarded) remain legitimately untyped. The IPC boundary is the desktop app's most type-fragile surface; without `<T>` TypeScript infers `unknown` and downstream `.field` access yields runtime `undefined` while compiling cleanly. Categorization walks back through whitespace + optional `await` to inspect the preceding token (`=`, `(`, `,`, `?`, `:`, or the `return` keyword = consumed). Sub-second on ~320 call sites.
- Extended `scripts/check-command-return-types.sh` (iter 30) with **parameter snake_case discipline**. Each `#[tauri::command]` parameter name must be snake_case. Rust's built-in `non_snake_case` lint warns on the same shape, but it can be locally disabled with `#[allow(non_snake_case)]`; this gate stage defends against that. A camelCase Rust parameter would serialize into the JSON wire format as `someParam` (Tauri uses Rust identifiers verbatim) — leaking the implementation naming into the IPC contract. Awk param-scanner skips `self`, `&`-prefixed references, and `mut` prefix; tracks `<>`/`()` nesting depth so `State<App, MyState>` is not mis-split on its inner comma. All 359 commands today pass — pure regression-prevention.
- Refined iter 32's V*_SCHEMA reference count in `scripts/check-migrations.sh` to be **comment-aware**. Line-comments (`// …`, `/// …`, `//! …`) and block-comments (`/* … */`) are now stripped before counting `V*_SCHEMA` tokens, so a future `// historical note: V11_SCHEMA used to also cover the ledger shape` does not inflate the per-constant reference count and trigger a false-positive "re-use" failure. Today's codebase has zero such comments so no behaviour change; this closes the documented iter-32 false-positive risk.

### Fixed

- `archive_create` IPC command (`src-tauri/src/commands/archive_commands.rs`) was using `unreachable!()` for the impossible format-mismatch arm of its format-dispatch `match`. The earlier allow-list validation guarantees the arm is dead under correct logic, but if the two lists ever drifted apart the IPC handler would panic instead of returning a structured error. Replaced with a defensive `Err(...)` of the same `Box<dyn std::error::Error>` type. Discovered by the iter 29 panic-macro extension to `scripts/check-command-unwraps.sh`.
- `safety_assess_intent` and `safety_confirm_intent` IPC commands (`src-tauri/src/commands/safety_commands.rs`) were declared `Result<T, ()>` — opaque error type that gives the frontend nothing to render. Tauri requires async commands with `State<'_, _>` references to return `Result`, so a bare-T return is not available; changed to canonical `Result<T, AppError>` and updated docs. Bodies remain fail-open (`Ok(...)`); the new shape leaves room for future structured error paths. Discovered by the iter 30 `ret-types` check.
- 6 user-facing env vars were referenced in code but missing from every `.env.example` template — fresh clones had no signpost for them: `ADMIN_DB_PATH` and `ADMIN_EMAIL`/`ADMIN_PASSWORD` (bootstrap-admin seeding in `admin/src/app/api/auth/login/route.ts`), `NEXT_PUBLIC_APP_NAME` and `NEXT_PUBLIC_API_URL` (admin client-bundle config), `TAURI_DEV_HOST` (Vite/Tauri dev-server LAN bind). Added entries to `admin/.env.example` and root `.env.example` with documentation comments. Discovered by the iter 31 `env-example` check.
- `handleGetInfo` callback in `src/components/file-manager.tsx` was assigning the result of `tauriInvoke("get_file_metadata", …)` (no type arg) to a `Record<string, any> | null` variable — TypeScript inferred `unknown` from the IPC call and the assignment silently degraded type safety on every downstream `.field` access of the metadata payload. Added explicit `<Record<string, unknown>>` type argument matching the variable's declared shape. Discovered by the iter 34 `invoke-types` check.

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
