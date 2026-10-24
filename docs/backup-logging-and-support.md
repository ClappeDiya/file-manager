# Backup notifications, logging and support tools

## Notifications

A job sends its run report through HTTP callbacks (**Hooks → HTTP reports**) and through these channels (**Hooks → Email, chat and webhooks**), plus any reusable notification profiles it chooses. Profiles, webhooks with presets (Pushover, ntfy, Discord, PagerDuty, Gotify), custom and secret headers, and HTML email are described in [backup-notifications.md](backup-notifications.md).

| Channel | Settings | Secret | Verified with |
|---|---|---|---|
| Email (SMTP) | server, port, STARTTLS / implicit TLS / none, username, from, to, subject | SMTP password | aiosmtpd with STARTTLS and AUTH |
| Telegram | chat id, Bot API URL | bot token | API-level tests |
| XMPP | sender JID, recipient, server, port, TLS mode | account password | Prosody with STARTTLS and SASL |
| Slack, Teams | none | webhook URL | API-level tests |
| Webhook | URL, method, content type, headers, body template, preset | `{{secret}}` value | local mock server per preset |

Each channel has its own result filter and message prefix. It also has its own certificate policy: accept invalid certificates, or trust a CA / pinned certificate given as PEM. **Send test** delivers a test message straight away.

Secrets are kept in the secure store under the channel's `credential_id`, never in the job.

Report options:

- **Hide file paths.** Source folders become `[source:…]` tags; other paths keep only the file name.
- **Lowest log level**, **log lines included** and **first retry delay**.
- A trusted CA for the report URLs.

## Log outputs

**Hooks → Log outputs** writes each run's log to:

- **a file**, rotated (one `.1` copy kept) above a size limit;
- **the system log**: the Windows Event Log (source `UFOP`) or syslog.

Each output has a minimum level, include and exclude tags (`engine`, `script`, `replica`, `report`, `network`, …) and a regex filter.

The CLI has `--log-level <error|warn|info|debug|trace>` for the console (it overrides `-v`; `RUST_LOG` still wins), and `--log-file <path>` with `--log-file-level` for a file. Values of secret-looking options (`--passphrase`, `--token=…`) are masked in the log.

## Diagnostics

**Maintenance → Diagnostics** checks four things:

- the job database: SQLite integrity and block references;
- the database against the destination listing: volumes missing at the destination, size mismatches, and files the database does not track;
- DNS, TCP connect and TLS handshake times to the service;
- upload and download speed, using a temporary test file that is read back, verified and deleted.

The CLI has `ufop backup db check <db>`.

## Bug reports

**Maintenance → Bug report** writes a zip with these files:

| File | Contents |
|---|---|
| `database.sqlite` | the job database. Every path component and symlink target is replaced by a salted hash (extensions kept). ACL and xattr metadata, job-config snapshots and change-journal state are removed. |
| `job.json` | the configuration. Passwords, tokens, keys and headers are blanked, URLs are cut to scheme and host, and email addresses and paths are hidden. |
| `runs.json` | the last 20 runs, sanitized the same way. |
| `job.log` | the last MiB of the job's log file, if it has one, with paths redacted. |
| `system.json` | app version, OS, CPU count, destination kind, and database schema and integrity. |

The CLI has `ufop backup db sanitize <db> <out>`, which writes only the sanitized database.

## Sync run history

Every sync run is stored in the app database (`sync_runs`, migration v16), keeping the newest 500 per pair. **Sync → History** lists all pairs; **Reports** on a pair lists that pair's runs. CSV and JSON exports work for stored runs too. The commands are `get_sync_run_history` and `prune_sync_run_history`.

Each sync run also stores its trigger (`run_trigger`: manual, scheduled, watcher, api) and, in `sync_run_items`, up to 10,000 changed files (copied, updated, deleted, moved, failed, conflict; failures first when the list is cut, `items_truncated` set). Commands: `get_sync_run_history` (optional `date_from`/`date_to`), `get_sync_run_items`, `get_sync_live_files` (running syncs). Backup runs store the trigger (`backup_runs.run_trigger`, migration v37) and one row per destination in `backup_replica_runs` (`backup_replica_history`; `backup_run_history` also takes dates).

## Application log files

The app writes `ufop.<date>.log` (human) and `ufop.<date>.jsonl` (one JSON object per line: time, level, source, message, fields) to the log folder. The in-app log viewer (`query_logs`) reads the JSON lines, falls back to the human file for days that have no `.jsonl`, and pages newest-first with a cursor. The retention setting prunes both kinds.

## Feedback

Help › Send feedback (`feedback_destination`, `feedback_submit`) sends a message, category and optional scrubbed diagnostics bundle to the registered management console as a bundle containing `feedback.json`; with no console it saves a zip to a folder. The result says which happened.

