# Kopia parity — implementation checklist

Source: the Kopia gap analysis (79 rows decided ADD, ENHANCE or DEFER). IDs are `K<row>` from that report.
Branch: `feat/kopia-parity`. One commit per item (or tight group), message prefixed with the ID.

Ground rules
- Additive only: existing behaviour, IPC names, CLI flags and v1 repositories keep working.
- New serde fields use `#[serde(default)]`; TypeScript mirrors use `T | null`.
- Format-changing features (CDC, zstd, envelope key, dlist compression, sharding) apply to format v2 repositories only.
- Advanced-only UI goes behind `appMode === "advanced"` (K132). Standard view stays as it is.
- Every item ships with tests; a box is ticked only when code, wiring (engine → IPC/CLI/API → UI where applicable) and tests are done and passing.

## WP0 — Foundations
- [x] K132 Backup screens honour Simple/Advanced mode (`AdvancedOnly` gate + store selector)
- [x] K130 SECURITY.md (disclosure policy, supported versions, response SLA)

## WP1 — Repository format v2 (ufop-core)
- [x] K72 Format v2, dual-version reader, in-place upgrade with v1 header backup and rollback
- [x] K83 Envelope key: random master key wrapped by Argon2id KEK; password change rewraps header only
- [x] K49 Zstd block codec in the native pack format (default for new v2 repos)
- [x] K52 Compress dlist/dindex metadata before sealing (v2)
- [x] K3 Content-defined chunking (FastCDC) behind a `Chunker` trait; chunker recorded in repo header
- [x] K82 ChaCha20-Poly1305 as a registered encryption module (v2 header records cipher)
- [x] K59 Optional sharded remote layout for v2 repos (+ reshard tool)

## WP2 — Repository safety
- [x] K115 Remote repository lock lease (`{prefix}-lock`), renew, stale takeover, guarded force-unlock
- [x] K118 Clock-skew check against backend mtime; warn/abort thresholds
- [x] K9 Periodic checkpoints (sealed partial dlist every N minutes / bytes)
- [x] K70 Read-only connections (Backend wrapper refusing writes; job + adopt flag)
- [x] K81 Encrypted local database for encrypted repositories (key in OS keychain)

## WP3 — Versions, retention, shared repositories
- [x] K7 Version notes and tags (+ tag filter)
- [x] K22 Pin versions ("Keep forever"), honoured by retention and delete
- [x] K113 Storage added per version (bytes_added, new_blocks)
- [x] K23 Retention preview (dry-run of deletions)
- [x] K20 Count-based GFS retention (`RetentionPolicy::Counts`)
- [x] K76 Copy selected versions to another repository
- [x] K4 Shared multi-client repositories (cross-job/machine dedup, DB sync from remote dindex)
- [x] K69 Client identity (host/user) recorded per version, override option

## WP4 — Sources and backup runs
- [x] K26 `.ufopignore` and optional `.gitignore` support
- [x] K28 Stay on one filesystem
- [x] K12 Fail-fast error policy
- [x] K29 Per-type read-error policy (file / directory / unknown entry)
- [x] K11 Percentage rehash
- [x] K6 Backup from stdin
- [x] K10 Per-run upload cap
- [x] K54 Large-file parallel hashing threshold
- [x] K31 ZFS and btrfs snapshot providers
- [x] K14 Pre-backup estimate (new bytes + duration)
- [x] K5 Run all jobs (CLI, IPC, tray)

## WP5 — Restore and access
- [x] K41 Restore as of a date/time
- [x] K42 Restore to ZIP / TAR / TGZ
- [x] K45 Mirror restore (delete extras, safety-interlocked)
- [x] K46 Restore ownership (chown when privileged)
- [x] K47 Restore error policy (shared with K12/K29)
- [x] K48 Sparse-file restore
- [x] K38 Read/preview a backed-up file without restoring
- [x] K77 Bounded local volume cache (size limit, stats, clear)
- [x] K44 Mount a version as a read-only WebDAV drive

## WP6 — Policy, schedules, maintenance
- [x] K17 Named policy templates inherited by jobs
- [x] K18 Effective-settings view (inherited vs overridden)
- [x] K33 Cron schedules for backup jobs
- [x] K116 Separate maintenance schedule (compact / verify)
- [x] K119 Verification scope (version/path), percent sampling, parallelism, max errors
- [x] K34 Async (fire-and-forget) hook mode

## WP7 — Storage backends
- [x] K60 S3 credential chain (env, profile, web identity, IMDS) for backup S3 store
- [x] K61 STS assume-role in native S3 store
- [x] K62 Azure service principal / federated auth for Azure store
- [x] K63 GCS read-only scope for read-only connections
- [x] K64 rclone process control (startup timeout, kill on drop)
- [x] K66 Storage class per volume type
- [x] K67 Point-in-time read of a versioned S3 bucket
- [x] K55 Request-rate limits and listing parallelism
- [x] K58 Live-provider contract test harness (env-gated) + CI workflow

## WP8 — Server, security, observability
- [x] K97 Graceful server shutdown (drain, checkpoint)
- [x] K89 CSRF double-submit token on cookie routes
- [x] K93 API tokens scoped to job IDs
- [x] K107 Request tracing with X-Request-Id
- [x] K108 Prometheus `/metrics`
- [x] K109 OpenTelemetry trace export + Prometheus pushgateway
- [x] K92 Multi-user server accounts and per-job ACL
- [x] K87 Ransomware hardening guide + destination checklist in job editor

## WP9 — Notifications
- [ ] K101 Reusable notification profiles
- [ ] K104 Webhook custom headers + presets (Pushover, ntfy, Discord, PagerDuty)
- [ ] K105 HTML email (multipart)

## WP10 — CLI, tooling, packaging
- [ ] K56 `ufop backup bench` (compression, crypto, chunker)
- [ ] K111 `ufop backup debug` (blocks, volumes, index, filesets)
- [ ] K129 Shell completions
- [ ] K125 Windows release targets
- [ ] K126 Multi-arch Docker image
- [ ] K127 Signed SHA256SUMS on releases
- [ ] K131 Portable mode for the desktop app
- [ ] K124 ufop-server web UI branding

## WP11 — Interface polish
- [ ] K122 Sort and paginate backup version/file tables
- [ ] K121 Text size setting
- [ ] K123 Units (binary/decimal) and schedule timezone

## Follow-ups found during the work
- [ ] Standalone `ufop agent` host honours cron and maintenance schedules of managed jobs (WP6 left it on plain intervals)
- [ ] Fix pre-existing `tests/journey_06_lan_transfer.rs` build break (`PeerManager::create_hello`/`create_ping`)
- [ ] Fix pre-existing orphan `test_automation_rule` IPC call (dump-ipc-commands --verify)

## Final verification
- [ ] ufop-core tests (all features) pass
- [ ] src-tauri `cargo test --lib` passes
- [ ] CLI tests pass
- [ ] Web vitest, tsc, eslint (0 errors) pass
- [ ] Docs updated (backup docs, cli-reference, api-reference, CHANGELOG)
