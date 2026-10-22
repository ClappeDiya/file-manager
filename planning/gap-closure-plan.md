# Gap-Closure Implementation Plan

Source: the Files.com gap analysis (154 rows, 441 catalog items). Scope: every row
decided **ADD**, **ENHANCE** or **DEFER**. KEEP rows stay untouched, and REJECT rows
are out of scope. Branch: `feat/gap-closure`, one PR per workstream, merged in order.

## Ground rules

1. **No regression.** Before each workstream, record the baseline:
   `cargo test --lib` (src-tauri), `cargo test` (crates/ufop-core, cli), `pnpm test --run`,
   `pnpm --filter admin test --run`, `pnpm lint`, and `tsc --noEmit` (root and admin).
   The workstream merges only when every suite is at least as green as its baseline.
2. **Reproduce, then fix.** Every P0/bug item starts with a failing regression test (CLAUDE.md).
3. **Backward compatible.** New serde fields get `#[serde(default)]`. New tables come
   from versioned migrations (and both test assertions for the migration count get
   updated). Existing IPC command signatures don't change; new behaviour arrives as
   new commands or optional args.
4. **DRY, modular.** One shared module per concern, never a copy:
   - `ufop-core::tokens`: path/rename token and regex engine, used by batch rename,
     automation, sync naming and archive naming.
   - `ufop-core::retry`: one backoff policy for transfer, automation, webhooks and SIEM.
   - `ufop-core::policy`: filename/extension policy, evaluated by the safety interlock,
     sync planner, automation and fleet.
   - `ufop-core::stability`: changing-source guard, used by sync and watcher triggers.
   - `admin/lib/delivery`: one outbound delivery queue (email, webhook, Slack, Teams,
     SIEM) with retry, signing and a delivery log.
   - `admin/lib/pagination`: one cursor/limit convention for every list route.
5. **Views.** Standard features appear in both app modes. Advanced features render
   only when `appMode === "advanced"` (existing gate). Admin features live in the
   console, CLI features in `ufop`.
6. **Verification.** Every item ships with unit tests and, where it has UI, a component
   test. Each workstream ends with a manual drive of the real flow (dev server or
   headless `ufop-server`), with the evidence recorded in the tracker.
7. **Honesty.** A UI stub is either wired or hidden behind a "Coming soon" flag. Never
   leave a stub that reports success.

## Workstreams (dependency order)

| WS | Name | Depends on | Priority mix |
|----|------|-----------|--------------|
| W0 | Baseline & hygiene | — | — |
| W1 | P0 correctness | W0 | P0 |
| W2 | Shared core modules (tokens, retry, policy, stability) | W1 | P1 |
| W3 | Automation engine v1.5 | W1, W2 | P1–P3 |
| W4 | Files, search & desktop UX | W1, W2 | P1–P3 |
| W5 | Connectors & network | W2 | P1–P3 |
| W6 | Desktop platform, fleet & agent | W2 | P1–P3 |
| W7 | Console delivery & alerting | W1 | P1–P2 |
| W8 | Console identity & security | W1 | P1–P3 |
| W9 | Console audit, reporting & data tables | W7 | P1–P3 |
| W10 | Billing | W8 | P1 |
| W11 | CLI, MCP & API surface | W2, W8 | P2–P3 |
| W12 | Deferred features | all above | P3 |

The live checklist is in the tracker artifact. Item IDs below match the tracker.

### W0 Baseline & hygiene
- W0.1 Record test, lint and typecheck baselines for all 5 packages
- W0.2 Disk hygiene (session disk is ~97% used): share one CARGO_TARGET_DIR, prune stale targets

### W1 P0 correctness
- W1.1 Automation executor dispatches RunSync / TransferToConnector / ExecuteCustomCommand / Notify for real; CompressFolder via archive engine; CreateFolder action
- W1.2 Cron `Schedule` rule triggers actually run (reuse the sync_engine cron scheduler)
- W1.3 S3 connector ListObjectsV2 continuation pagination (+2-page mock test)
- W1.4 SFTP known_hosts persisted and system `~/.ssh/known_hosts` read; mismatch test
- W1.5 Tags/labels/color tags persisted (migration) with key-value metadata
- W1.6 Console stubs: feature-flag "Coming soon" for any not yet wired; persist AI-governance settings + audit (hash-chained)

### W2 Shared core modules
- W2.1 `tokens` engine extracted from batch-rename (date/ext/name/counter/regex capture), with a TS mirror for live preview
- W2.2 `retry` policy unified (transfer RetryPolicy becomes the shared type)
- W2.3 `policy` engine: extension allow/block plus regex name rules, inheritance-ready
- W2.4 `stability` guard: size/mtime stable for N seconds

### W3 Automation engine v1.5
- W3.1 Route tokens in dest_path / rename (rows 92, 93, 193)
- W3.2 Retry with backoff + attempts in AutomationLog; resume failed run (199, 216)
- W3.3 Run-command action with timeout, templated args, interlock (213)
- W3.4 Multiple destinations per rule (192)
- W3.5 Per-rule overwrite policy, reusing ConflictPolicy (194)
- W3.6 Per-rule no-overlap queue + priority ordering (195, 96)
- W3.7 Condition groups All/Any/Not + ordered multi-step actions with on-error (205, 215)
- W3.8 Notify: OS notification + email/Slack/Teams via notify.rs (212, 248)
- W3.9 OlderThan condition + expire to trash through interlock dry-run (99)
- W3.10 ExtractArchive action with member cap (108)
- W3.11 Newest-N selection (191)
- W3.12 Clone rule (198)
- W3.13 Inbound webhook trigger on ufop-server with scoped token (189)
- W3.14 Folder-activity notification presets (244, 246)
- W3.15 Changing-source guard on watcher triggers (172, shared with W4.12)

### W4 Files, search & desktop UX
- W4.1 OS drag-and-drop into panes (2)
- W4.2 Unified Operations drawer (fs ops + transfer + sync + archive + automation) (6, 399)
- W4.3 SQLite FTS5 index (local + cached remote listings) and real content search (11, 140)
- W4.4 Grid thumbnails with LRU cache (1)
- W4.5 Copy to multiple destinations (3)
- W4.6 SHA-1 + CRC32 checksums (9)
- W4.7 CSV/JSON table preview (19)
- W4.8 Editor autosave + draft recovery (22, 25)
- W4.9 Provider share links with expiry + org policy flag (27, 29, 33, 41, 43, 142)
- W4.10 GPG file encrypt/decrypt/sign, armor, key list + expiry (227, 229, 230, 232)
- W4.11 Sync: Move-after-copy UI, completion marker, version-naming template (69, 170, 171, 101, 102)
- W4.12 Sync changing-source guard (172)
- W4.13 Extension/regex policy enforced in interlock + sync + transfer (88, 90)

### W5 Connectors & network
- W5.1 Staged temp-name upload + rename for SFTP/FTP/WebDAV (139, 166)
- W5.2 Proxy: system/PAC/manual on all HTTP clients; fleet-settable (70, 180)
- W5.3 STS AssumeRole with external ID + refresh (145)
- W5.4 Rclone browse connector (GCS/Box/70+) registered in ConnectorRegistry (149, 156, 283)
- W5.5 S3 presets R2/Linode/Filebase; SFTP presets Egnyte/ShareFile (150, 152, 153, 162)
- W5.6 Cloud mounts via `rclone mount` + capability badge (66, 138)

### W6 Desktop platform, fleet & agent
- W6.1 Rolling log files + retention; "Send diagnostics" to console (185)
- W6.2 Desktop SSO sign-in to console via loopback OIDC (71)
- W6.3 ManagedPolicy app settings: update channel, blackout window, proxy, AI toggles; silent-install docs (72, 183, 184)
- W6.4 Agent root confinement / read-only / symlink policy (176)
- W6.5 Agent global bandwidth + concurrency caps (179)
- W6.6 Central credential versions + rotation propagation (136)

### W7 Console delivery & alerting
- W7.1 `delivery` service: persisted webhooks (replaces the in-memory stub), headers, HMAC, retry/pause, log, test (256, 300–306)
- W7.2 SMTP outbox with retry + delivery status page (82, 400)
- W7.3 Teams channel kind (248)
- W7.4 Expected-arrival alerts + ack/snooze (236–239, 243)

### W8 Console identity & security
- W8.1 SCIM 2.0 Users/Groups + per-org tokens (332, 335, 340)
- W8.2 Bulk CSV user import, server-side (310)
- W8.3 Password strength, optional expiry, self-service reset (358–360)
- W8.4 Org-wide MFA enforcement (364)
- W8.5 WebAuthn/passkeys (362)
- W8.6 Sessions table: idle timeout + per-session revoke (368, 369)
- W8.7 CIDR IP allowlist (console + ufop-server) (372)
- W8.8 Step-up re-auth for sensitive routes (378)
- W8.9 Scoped, expiring API tokens (ufop-server + console) (285, 307)
- W8.10 HSTS + min TLS on ufop-server (374, 381, 382)
- W8.11 Inactive credential retirement (366)
- W8.12 Soft delete/restore, expiry, offboarding reassignment (325–329)
- W8.13 User time zones (322)
- W8.14 Effective-access "Explain" panel (318, 319)
- W8.15 Policy inheritance with lock flag (355)

### W9 Console audit, reporting & data tables
- W9.1 SIEM forwarder (syslog/HEC/HTTPS/JSONL), cursor + retry + backlog (402–404)
- W9.2 AI transcript audit on the hash chain (269)
- W9.3 API request log + SSO diagnostics (396, 397)
- W9.4 Report catalog + scheduled emails + run status (406–411)
- W9.5 Per-log-type retention with anchored pruning (412)
- W9.6 Background export jobs (390)
- W9.7 Shared pagination convention across routes (299)
- W9.8 Data table column picker + server paging (87)
- W9.9 Admin console i18n (84)

### W10 Billing
- W10.1 Billing provider abstraction + Stripe Checkout/portal + webhook → license seats (430, 431, 433, 441)

### W11 CLI, MCP & API surface
- W11.1 CLI named profiles (292)
- W11.2 CLI CSV output (294)
- W11.3 `ufop mcp` stdio server with root confinement (295, 297)
- W11.4 OpenAPI for ufop-server (298 follow-up)

### W12 Deferred features (last, each scoped as a real minimal version)
- W12.1 Metadata schemas & validation (14)
- W12.2 Office previews via OS handoff (18)
- W12.3 Hosted share portal module in ufop-server (28–45 subset)
- W12.4 Offline cache + mount-profile policy (67, 73)
- W12.5 Multi-session (RDS) validation + fixes (74)
- W12.6 Org branding in console + email templates (77–81)
- W12.7 Folder lock / policy inheritance / copy config / intake order for server-hosted folders (91, 94–96)
- W12.8 Legal hold on backup sets + eDiscovery export bundle (104, 105)
- W12.9 Read-only WebDAV serve in ufop-server (118–127 subset)
- W12.10 ADLS Gen2 + Azure Files connectors (147, 148)
- W12.11 Gateway agent features (174, 177, 178, 181, 182): scope review, then minimal relay
- W12.12 URL download action (197)
- W12.13 Reusable schedules + holiday calendars (201, 202)
- W12.14 JSON workflow DAG + visual builder (203–209, 217–219)
- W12.15 Document text extraction to the FTS index (214)
- W12.16 GPG recrypt in memory, legacy MDC, key generation (228, 231, 233); review the symmetric temp plaintext
- W12.17 Image resize/convert action; document conversion (234, 235)
- W12.18 Batch validation rules + reports (240–242)
- W12.19 AI reports, local docs help, unattended AI tasks (260, 261, 265–268)
- W12.20 Generated SDKs; policy-as-code export (287, 290)
- W12.21 MCP OAuth (296)
- W12.22 Read-only "view as" (324)
- W12.23 LDAP/AD (337)
- W12.24 Malware-scan preset (376)

## Verification matrix (per item)

| Layer | Tool |
|---|---|
| Rust engine | `cargo test` unit + integration (tempdir fixtures, mock HTTP servers) |
| IPC | command-level tests via managers; `tauriInvokeSafe` fallbacks |
| Frontend | Vitest + Testing Library; stores reset in `beforeEach` |
| Console | Vitest route tests (admin/src/__tests__ helpers) on an in-memory SQLite |
| End-to-end | `ufop-server` headless + curl; Vite dev server + Playwright for UI flows |
