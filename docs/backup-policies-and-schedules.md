# Backup policies, schedules and maintenance

This page covers how backup jobs share settings and when their work runs:

- named policy templates that jobs inherit from
- the effective-settings view: where each setting comes from
- cron schedules for backups
- the time zone a schedule runs in
- separate schedules for compaction, verification and retention
- verification scope, sampling, parallelism and an error limit
- asynchronous (fire-and-forget) hook scripts

Everything here except the plain "Check backup" buttons is in **Advanced** mode. Restores are in [backup-restore.md](backup-restore.md), locks in [backup-safety.md](backup-safety.md), retention rules in [backup-versions.md](backup-versions.md).

## Policy templates

A **policy template** is a named set of job settings. It holds only the settings you put in it; anything it leaves out is not touched. A job can be **based on** several templates, in order. A later template wins over an earlier one.

Every setting a job runs with is decided by one function (`backup_engine::templates::resolve`), in this order. Each layer wins over the one before it:

```text
built-in defaults  <  global defaults  <  templates (in the job's order)  <  the job's own settings  <  organization (enforced)
```

- **Global defaults** are the existing *Settings → Defaults for all jobs* (backup options, network limits and retries).
- **The job's own settings** are the ones the job sets itself, listed in its `overrides`. Every other setting is inherited. If a template changes later, every job that inherits that setting follows the change.
- **Organization templates** come from the management console (see below). An **enforced** one applies to every job and wins over the job's own settings.
- Some options always belong to one job and are never inherited: `prefix`, `block_size`, `block_hash`, `file_hash`, `dry_run`, `task_config`, `changed_paths` and `deleted_paths`. Templates and global defaults refuse them.

### What a template can set

A setting is named by a **field path**:

| Field path | What it sets |
|---|---|
| `options.<name>` | One backup option, e.g. `options.compression_level`, `options.upload_concurrency`, `options.verify_samples` |
| `filter` | Include/exclude rules and ignore files (the whole section) |
| `retention` | The retention policy |
| `schedule` | The backup schedule (interval or cron) |
| `maintenance` | The maintenance schedules (see below) |
| `scripts` | Hook scripts and their modes |
| `notifications`, `notify_on` | Email/chat/webhook notifications and which results raise a desktop notification |
| `notification_profile_ids` | Notification profiles the job sends to (see [backup-notifications.md](backup-notifications.md)) |
| `throttle`, `retry` | Network limits and retries |
| `prevent_sleep` | Keep the computer awake while the job runs |

Template JSON (`PolicyTemplate`):

```json
{
  "name": "Laptops",
  "description": "Nightly, keep 30 days",
  "settings": {
    "options": { "compression_level": 3, "upload_concurrency": 2 },
    "retention": { "type": "keep_days", "days": 30 },
    "schedule": { "enabled": true, "cron": ["30 1 * * *"] }
  }
}
```

### Jobs from before templates

Jobs saved before templates existed have no `overrides` list (`null`). For them, every setting that differs from the built-in default counts as the job's own. That is the same rule global defaults always used, so these jobs run exactly as before. When such a job gets its first template, the app saves that list for it once. The job keeps what it had set, and everything still at the default is inherited from then on. Older job JSON stays valid. The new fields `template_ids`, `overrides` and `maintenance` are all optional.

### Where to manage templates

| Where | |
|---|---|
| App | *Settings → Policy templates*: create, edit (settings as JSON), delete, export all, import. Organization templates are listed read-only and can be copied. In the job editor's *General* step, *Based on templates* adds, reorders (↑ ↓) and removes templates. |
| CLI | `ufop backup template list \| show T \| create --file t.json \| create --name N --settings '{…}' \| update T [--file …] [--name …] [--settings …] \| delete T \| apply T --job J \| export [--template T] [--out f.json] \| import f.json`. These commands use the signed-in `ufop-server` (`ufop server login`), like `ufop backup run-all`. `T` and `J` are an id or a name. |
| REST | `GET`/`POST /api/v1/backup-templates`, `GET`/`PUT`/`DELETE /api/v1/backup-templates/{id}`, `POST /api/v1/backup-templates/{id}/apply` with `{job_id}`, `GET /api/v1/backup-templates/export?ids=a,b`, `POST /api/v1/backup-templates/import` (body: the file) |
| IPC | `backup_list_templates`, `backup_get_template`, `backup_save_template { template }`, `backup_delete_template { id }`, `backup_apply_template { templateId, jobId }`, `backup_export_templates { ids? }`, `backup_import_templates { json }` |

Rules:

- Names are unique, ignoring case.
- A template that a job uses cannot be deleted. The error names the jobs.
- Importing updates a template that has the same id or name, and adds the rest.
- A job's export file (*Maintenance → Export configuration*) includes the templates it is based on. Importing it creates any that are missing, or reuses a local template with the same name.

### Organization templates

The management console's **App settings** page has a *Backup policy templates* section. Each template has a name, an id, settings JSON and an *Enforced* switch. The templates are sent to agents with the other managed settings (`managedSettings.backupTemplates`, `ufop_core::agent::ManagedBackupTemplate`).

- The desktop app and `ufop-server` install them read-only, with ids `org:<id>`.
- An ordinary organization template can be chosen in a job.
- An enforced template applies to every job and cannot be chosen.
- The console and the agent both refuse templates that set `scripts`, because running programs is a decision for each desktop.
- The desktop checks the settings in full and reports any template it refuses.
- Stopping management removes the organization templates.

## Effective settings

The job editor's *General* step has an **Effective settings** panel in Advanced mode. It lists every setting the job runs with, its value, and a badge that shows where the value comes from: *Default*, *Global defaults*, *Template: name*, *This job* or *Organization: name*.

- The panel shows the job as it is in the editor, so it updates before you save.
- For settings the job sets itself, **Reset to inherited** goes back to the inherited value. The button's tooltip shows that value and where it comes from.
- When a job is based on templates, the editor shows the effective values. Any setting you change becomes the job's own.
- Warnings name templates that no longer exist. Their settings are skipped.

| Where | |
|---|---|
| CLI | `ufop backup effective <job>`: a table of setting, source and value. `--json` gives the full list. |
| REST | `GET /api/v1/jobs/{id}/effective-settings` returns `{job_id, template_ids, overrides, settings: [{field_path, value, source: {type: built_in\|global\|template\|job\|managed, id?, name?}, inherited: {value, source} \| null}], warnings}` |
| IPC | `backup_effective_settings { id }` for a saved job, or `{ job }` for an unsaved one. `backup_effective_job { id }` returns the resolved job itself. |

## Cron schedules for backups

A schedule (`BackupSchedule`) works in one of two modes:

- **Start time and repeat** (the default): a start time, a repeat interval and weekdays.
- **Custom (cron)** (Advanced): one or more cron expressions in `cron`. When `cron` is not empty, it replaces the start time, interval and weekdays. The next run is the earliest match of any expression.

| Form | Example | |
|---|---|---|
| 5 fields | `30 2 * * Mon-Fri` | `minute hour day month weekday`, as in crontab. Runs at second 0. |
| 6 fields | `0 30 2 * * Mon-Fri` | With seconds first |
| 7 fields | `0 0 3 1 * * 2027` | With a year last |

- Expressions are read by the schedule library's cron parser (`ufop_core::backup::schedule::parse_cron_any`, the same `cron` crate that sync pairs and automation rules use). Named schedules (`schedule:<id>`) are not accepted here. Paste their expression instead.
- Times are **local** unless the schedule has a [time zone](#time-zone). A local run at 02:30 stays at 02:30 when daylight-saving time changes.
- Validation errors are written to be readable, e.g. *Invalid cron expression '61 \* \* \* \*': … Use `min hour day month weekday`*. A schedule that would run more often than every 5 minutes, or never, is refused.
- **Missed runs** behave as they do for interval schedules. If the app was closed when one or more runs were due, the backup runs once as soon as the scheduler is back. It does not run once per missed slot. Holiday calendars and *Skip while on battery* apply to cron schedules too.
- The editor shows the **next five runs** and strikes out runs that a holiday calendar skips.
- The fleet console's overdue check uses the gap between the next two runs as the job's interval.

| Where | |
|---|---|
| Job JSON | `"schedule": {"enabled": true, "cron": ["30 2 * * Mon-Fri", "0 9 * * Sat"]}` (REST `POST`/`PUT /api/v1/jobs`, templates) |
| Preview | `POST /api/v1/backup-schedule/preview` with `{schedule, count}` (needs the `read` scope); `ufop backup schedule-preview --cron "30 2 * * *" [--cron …] [--count 5]` (or `--start-time 03:00 --interval-minutes 60`); IPC `backup_schedule_preview { schedule, count }` |

### Managed jobs on the standalone CLI agent

`ufop agent run` uses the same schedule code as the desktop app and `ufop-server` (`ufop_core::backup::schedule`), so a managed job runs on the same slots everywhere:

- interval and cron schedules, in the schedule's [time zone](#time-zone), with one catch-up run for slots missed while the agent was stopped;
- compaction, verification and retention on their own [maintenance schedules](#maintenance-schedule) (each reported to the console with its task as the operation); a scheduled compaction or retention no longer also runs after each backup;
- settings inherited from the organization's **policy templates** (managed app settings `backupTemplates`): the policy's `template_ids` (`org:<id>`, in order) fill in what the policy leaves unset, enforced templates win over the policy. The resolution order is the desktop's (`ufop_core::backup::layers`).

The CLI agent applies the backup options, filter, retention, schedule and maintenance. Other sections a policy or template sets (scripts, notifications, throttle, retry, prevent sleep) are ignored with a warning in `ufop --json agent status`. Holiday calendars are not delivered to CLI machines: calendar references are dropped with a warning, so those days are not skipped. *Skip while on battery* is not checked. Jobs saved by older versions keep their plain interval until the console sends the policy again.

## Time zone

Each schedule (the backup schedule and each maintenance schedule) has a `timezone`. The start time, the weekdays, the cron fields and the holiday calendars are read in that zone, and so are the next-run preview, missed-run catch-up and the "today is a day off" check.

| Value | Meaning |
|---|---|
| `null`, `""` or `local` (default) | This computer's (or the server's) time zone. Follows daylight saving: 02:00 stays 02:00 all year. |
| `UTC` (also `GMT`, `Z`, `Etc/UTC`) | Coordinated Universal Time |
| `+02:00`, `-05:30`, `+0530`, `+2`, `UTC+1`, `GMT-3` | A fixed offset from UTC, −14:00 to +14:00. It does **not** change for daylight saving. |

Named zones such as `Europe/Berlin` are refused with an explanation: the app ships no time-zone database (the offline build has no `chrono-tz`), so it cannot follow another region's daylight-saving rules. Use `local` on a computer in that zone, or the fixed offset you want.

- **Editor** (Advanced): *Time zone* under the schedule, with this computer's zone, UTC and the common offsets. When a zone other than local is chosen, the next-run preview shows each run in that zone and the same moment on this computer, e.g. *Tue, Mar 17, 2026, 02:30 (UTC+02:00) — 3/17/2026, 1:30:00 AM on this computer*.
- **Job JSON / templates / REST**: `"schedule": {"enabled": true, "cron": ["30 2 * * *"], "timezone": "UTC"}`. Older jobs without the field run in local time as before. A job with an unknown zone is refused on save (`400` over REST).
- **Preview**: `POST /api/v1/backup-schedule/preview` and IPC `backup_schedule_preview` answer each run's `at` (UTC) and `zoned` (the same instant with the zone's offset). `ufop backup schedule-preview --timezone +02:00 …` prints the zone time and this computer's time.
- The run-history and version times in the Backup screens can be shown in UTC instead of local time: Settings → Appearance → *Backup times*.

## Maintenance schedule

By default, retention and compaction run after each backup. `auto_compact_interval_days` limits how often compaction runs. Verification samples `verify_samples` volumes after each backup.

A job can instead give each maintenance task its own schedule (`maintenance`). It uses the same schedule type as backups: interval or cron.

```json
"maintenance": {
  "compact":   { "enabled": true, "cron": ["0 3 * * Sun"] },
  "verify":    { "schedule": { "enabled": true, "cron": ["0 4 * * Sat"] },
                 "options": { "mode": { "type": "sample_percent", "percent": 10 }, "files_percent": 1 } },
  "retention": { "enabled": true, "cron": ["30 3 * * *"] }
}
```

- The scheduler starts each task as an ordinary job operation: `compact`, `verify` (with the task's verification options) or `apply_retention`.
- Each task takes the repository lock it needs: exclusive for compaction and retention, shared for verification.
- Each task appears in the run history, report and notifications like any other operation.
- A job runs one operation at a time, so a busy job's task waits for the next pass.
- **Pause** (tray, `POST /scheduler/pause`) pauses maintenance too. Holiday calendars and *Skip while on battery* apply.
- Read-only jobs never run maintenance.
- Missed slots run once, as they do for backups. The last start of each task is kept in `stats.last_maintenance`.
- **Mapping to the older settings:** when compaction has its own enabled schedule, backups no longer compact afterwards, and `auto_compact` and `auto_compact_interval_days` are ignored. When retention has its own enabled schedule, backups no longer apply retention. Tasks without a schedule keep the after-backup behaviour, so `auto_compact_interval_days` works as before. `verify_samples` still checks volumes after each backup.
- **Global default:** put `maintenance` in a template and base jobs on it, or use an enforced organization template.
- The scheduler status lists upcoming tasks in `next_maintenance`. Each entry is a `NextRun` with `operation` set.

In the app, the job editor's *Options* step has a **Maintenance schedule** section in Advanced mode. It has a schedule editor for each task and, for verification, the options described in the next section.

## Verification: scope, sampling, parallelism, error limit

Verification (`ufop_core::backup::verify`) checks at three levels. Each level includes the one before it.

1. **Structure** (always): every block that the scoped versions use is in the index, and every volume those blocks and the versions' file lists live in exists at the destination. This needs one listing and no downloads.
2. **Volumes**: download volumes, check each one's recorded SHA-256, then decrypt and unpack it. `full` downloads every volume in scope and also hashes every block.
3. **Files** (`files_percent`): rebuild that share of the scoped files from their blocks (download, decrypt, decompress) and compare each file with its recorded hash, as Kopia's `--verify-files-percent` does.

`VerifyOptions`:

| Field | | |
|---|---|---|
| `scope.versions` | `[]` = all | Version selectors: a number (0 = newest), `latest`, `oldest`, or a timestamp / file-list name |
| `scope.paths` | `[]` = all | Folders (everything below them) or wildcards (`*/Documents/*`) |
| `mode` | `{"type": "sample_volumes", "count": 1}` | `structure`, `sample_volumes {count}`, `sample_percent {percent: 1–100}`, `full` |
| `files_percent` | none | 0–100 |
| `parallelism` | 0 = 4 | Downloads and file checks running at the same time (at most 64) |
| `max_errors` | 0 = no limit | Stop after this many problems; `stop_reason` says so |
| `use_cache` | off | Read volumes from the local volume cache. By default verification always downloads, so it checks the destination itself. |

When the scope is *everything*, index volumes and unreferenced volumes are checked too. A sample always includes the newest scoped file list.

The report (`VerifyReport`) keeps the older fields `checked` (volumes downloaded) and `errors` (one line per problem). It adds:

- `problems`: each with a `kind` (`missing_block`, `missing_volume`, `volume`, `block_hash` or `file`), the `volume`, a `message`, and the `versions` and `paths` it affects (up to 20 paths)
- `versions_checked`, `files_in_scope`, `files_checked`, `blocks_referenced` and `volumes_in_scope`
- `stop_reason`

| Where | |
|---|---|
| App | *Maintenance → Integrity*: **Verify sample** and **Verify everything** work as before (Standard). In Advanced mode, **Custom check** sets the scope, volume sampling, file percentage, parallel downloads, error limit and cache use. |
| CLI | `ufop backup verify --dest … [--version latest] [--path '*/Docs/*'] [--structure \| --samples N \| --percent N \| --full] [--files-percent N] [--parallel N] [--max-errors N] [--use-cache] [--json]`. Exit code 2 when problems are found. |
| REST | `POST /api/v1/jobs/{id}/operations` with `{"type": "verify", "full": false, "options": {…}}` |
| IPC | `backup_execute` / `backup_start_operation` with the same operation |

The older form `{"type": "verify", "full": true|false}` still works. `full` means every volume with block hashes. Otherwise the job's `verify_samples` (at least 1) volumes are downloaded, as before. `maintenance::verify(repo, samples, full)` maps onto the new options.

## Asynchronous hook scripts

Hook scripts (*Hooks → Scripts*, Advanced) have a mode:

| Mode | Before backup | After backup |
|---|---|---|
| `required` | Waits; a failure or timeout aborts the backup | (as optional) |
| `optional` | Waits; a failure is a warning in the run log | Waits; problems are logged (the default) |
| `async` | Starts the script and backs up without waiting | Starts the script and finishes the run |

- An **async** script's output lines (`LOG:INFO`/`WARN`/`ERROR`) and exit status go to the run log if the operation is still running when the script ends. Otherwise they go to the app log (`ufop::backup`).
- The timeout (`timeout_secs`, 0 = 60 s) still kills the script in the background.
- An async post-script gets the same `UFOP_*` variables and result file as before. The file is removed when the script ends.
- Async scripts never block or fail the backup.

JSON: `scripts.before_mode` and `scripts.after_mode` are `"required" | "optional" | "async"`. When `before_mode` is not set, the older `run_before_required` decides between required and optional, so existing jobs keep their behaviour. The source-release script (`run_on_source_release`) always waits. The `ufop` CLI has no hook scripts, so this setting is only in the app, the API and templates. Organization templates cannot set scripts.
