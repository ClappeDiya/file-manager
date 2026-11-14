# Backup versions: notes, tags, pins, retention and copies

Each backup run creates a version. This page explains how to label versions, keep chosen ones forever, see what retention will delete before it happens, choose a daily/weekly/monthly schedule, and copy versions to another backup. Storage format details are in [backup-repository-format.md](backup-repository-format.md#version-metadata).

## The version list

**Restore → Versions** shows a table under the version picker:

| Column | |
|---|---|
| Time | Click to select the version for browsing and restore. *(partial)* marks an interrupted run or a checkpoint. |
| Items, Size | Files and folders in the version, and their total size |
| Added | Storage the version added to the destination: new block volumes and their indexes, after compression and encryption. Versions made before this was recorded show "—". |
| Keep | 📌 "Keep forever" (see below) |
| Note | Click to add or change a note. Enter saves, Esc cancels, an empty note removes it. |
| Tags | `key=value` labels. In Advanced mode click *+ tag* / *edit* and type `key=value, key2`. |
| Computer | *(Advanced)* `user@host` that wrote the version |

In Advanced mode, chips above the table filter by tag (several chips combine), and a menu filters by computer when versions come from more than one.

### Sorting and pages

Click a column header (or focus it and press Enter or Space) to sort by it; click again to reverse. The header's `aria-sort` tells screen readers the order. Below the table, **Rows per page** offers 25, 50, 100, 250 or All (All scrolls one list that only draws the visible rows), with First / Previous / Next / Last buttons. Each backup table remembers its page size and sort on this computer (with the other window preferences): the version list, the files of a version and the search results in Restore, the run history, and the destination files in Maintenance.

The files of a version and the run history are fetched one page at a time, sorted and filtered by the backend, so versions with hundreds of thousands of files open quickly; selected files stay selected while you page. For scripts, the same paging is available on the IPC calls `backup_list_files`, `backup_run_history` and `backup_remote_list` (optional `page: {offset, limit, sort, descending, filter}`, answered with `{items, total, offset, limit}`; without `page` they return the plain list as before) and on REST `GET /api/v1/jobs/{id}/files` and `GET /api/v1/runs` (`offset`, `limit`, `sort`, `desc`, `filter`; see [api-reference.md](api-reference.md)). Sort columns: files `path`, `size`, `kind`, `modified`; runs `started_at` (default, newest first), `finished_at`, `status`, `operation`, `job_id`; destination files `name`, `size`.

The CLI shows the same columns:

```bash
ufop backup list --dest /mnt/backup [--tag env=prod] [--json]
```

## Notes and tags

Set a note and tags for every new version of a job under **Options → Advanced → Versions and sharing**, or per run with `ufop backup run … --note "before upgrade" --tag env=prod --tag release`. You can change them later:

- the version table (Standard: notes; Advanced: tags)
- `ufop backup annotate --dest … <version> --note "…" [--tag k=v …] [--clear-tags]`
- `PATCH /api/v1/jobs/{id}/versions/{version}` with `{"note": "…", "tags": {"k": "v"}}`, `backup_set_version_metadata`, or the `set_version_metadata` operation

A change rewrites only that version's file list. Filter by tag with `--tag` (CLI), `?tags=env=prod,release` (REST) or the `tags` argument of `backup_list_versions`. A bare key matches any value.

## Keep forever (pins)

A pinned version is never removed by retention and never rewritten by *Remove files from history*. Deleting it asks for confirmation and uses `force`. A legal hold still applies on top: under a hold nothing is deleted at all.

- Click 📌 in the version table.
- `ufop backup pin --dest … <version> [--reason "audit"]` and `ufop backup unpin --dest … <version>`
- `ufop backup delete --dest … <version> --force` deletes a pinned version. Without `--force` it is refused.

## Retention preview

The job editor's **Retention** step lists the job's existing versions under the policy. It shows what the policy keeps after the next backup and why, for example "kept: newest version, daily 7", "kept: pinned" or "deleted: not selected by any rule". The list updates as you change the policy, before you save. The same decision function runs after each backup, so the preview is exactly what happens.

```bash
ufop backup retention preview --dest … --keep-daily 7 --keep-weekly 4
ufop backup retention apply   --dest … --keep-daily 7 --keep-weekly 4 [--dry-run]
```

The REST call is `POST /api/v1/jobs/{id}/retention/preview` with `{"policy": {…}}` (omit `policy` to use the job's). The IPC command is `backup_preview_retention`. The `apply_retention` operation applies a policy now. The newest version, pinned versions and versions with an active object lock are always kept. Under a legal hold the preview keeps everything and applying is refused.

## Daily, weekly, monthly and yearly versions

**Retention → Keep daily, weekly, monthly and yearly versions** keeps 7 daily, 4 weekly, 12 monthly and 3 yearly versions by default. In Advanced mode you can set each count, plus *latest* and *hourly*. The rule works like Kopia's. For each kind of period, the newest version in each of the N most recent periods that have versions is kept. A version can count for several rules. All periods are UTC, and weeks are ISO weeks starting on Monday. Setting every count to 0 keeps everything.

| Option | CLI | API (`{"type": "counts", …}`) |
|---|---|---|
| Latest N | `--keep-latest` | `keep_latest` |
| Hourly | `--keep-hourly` | `keep_hourly` |
| Daily | `--keep-daily` | `keep_daily` |
| Weekly | `--keep-weekly` | `keep_weekly` |
| Monthly | `--keep-monthly` | `keep_monthly` |
| Yearly | `--keep-yearly` | `keep_yearly` |

The older policies (keep all, by age, by count, smart, custom rules) are unchanged.

## Copy versions to another backup

**Restore → Copy to another backup…** (Advanced) copies the selected version, or every version shown, to another job's backup or to a folder. *Check first* is a dry run. Only data the other backup lacks is uploaded. Notes, tags, pins and the computer name are kept. If the other backup uses a different block format, for example format 1 and format 2, the data is split into blocks again.

```bash
UFOP_TO_PASSPHRASE=… ufop backup copy-versions --dest /mnt/backup 0 3 --to /mnt/offsite [--to-format-version 2] [--dry-run]
```

The operation `{"type": "copy_versions", "versions": [0], "target": {"type": "job", "job_id": "…"}, "dry_run": false}` does the same, or use `"target": {"type": "destination", "destination": {…}, "passphrase": "…"}`.

## Several computers in one backup

A shared repository takes backups from several computers or jobs, and identical data is stored once. Each job keeps its own versions and retention. See [Shared repositories](backup-repository-format.md#shared-repositories-and-client-identity). In the app:

- **Maintenance → Sharing** (Advanced) lists the computers and jobs and offers *Share this repository with other computers/jobs*.
- **Restore → Versions of …** (Advanced) switches between *This job*, another computer's versions and *All sources*. *Refresh* reads their latest versions from the destination.
- **Options → Advanced → Versions and sharing** records another computer or user name. Use it to continue a computer's history after moving to a new one.

Restoring as of a time, archives, mirror restores, previews, the volume cache and opening a version as a drive are described in [backup-restore.md](backup-restore.md). Templates, cron and maintenance schedules, and scoped verification are in [backup-policies-and-schedules.md](backup-policies-and-schedules.md).
