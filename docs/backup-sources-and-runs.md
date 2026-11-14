# Backup sources and runs

This page covers what a backup reads and how a run behaves:

- ignore files
- staying on one filesystem
- the read-error policy
- re-checking unchanged files
- backing up a command's output or standard input
- an upload limit per run
- parallel hashing of large files
- ZFS and btrfs snapshots
- the pre-backup estimate
- backing up every job at once

Format and safety topics are in [backup-repository-format.md](backup-repository-format.md) and [backup-safety.md](backup-safety.md).

## Ignore files (`.ufopignore`, `.gitignore`)

In every source folder the backup reads the files named in **Ignore-file names** (default `.ufopignore`). The rules in each file apply to its folder and everything below it. They use the `.gitignore` syntax:

| Pattern | Matches |
|---|---|
| `*.log` | a name at any depth (`a.log`, `x/y/a.log`) |
| `/build` | `build` in this folder only (a `/` at the start or in the middle anchors the pattern) |
| `docs/*.tmp` | `docs/x.tmp` below this folder; `*` and `?` never match `/` |
| `cache/` | folders named `cache` only |
| `**/logs`, `logs/**`, `a/**/b` | `logs` anywhere; everything inside `logs`; `a/b`, `a/x/b`, `a/x/y/b` |
| `file[0-9].c`, `n[!a]t` | character classes, negated with `!` or `^` |
| `!keep.log` | re-include what an earlier line excluded |
| `\#name`, `\!name`, `trailing\ ` | a literal `#`, `!` or trailing space |

Lines starting with `#` are comments, and blank lines are skipped. An invalid line, such as an unclosed `[`, is ignored, as git does.

**Precedence:**

1. **Explicit filter rules come first.** These are the include and exclude rules of the job (first match wins). An explicit include brings back a file that an ignore file excludes. An explicit exclude always wins.
2. **Ignore files** decide for entries that no explicit rule matches:
   - A file deeper in the tree overrides the files in the folders above it.
   - Within one folder the order is `.git/info/exclude`, then `.gitignore`, then the ignore files in the order they are listed. A later line overrides an earlier one.
   - A folder that is excluded is not entered, so nothing inside it can be re-included. Git works the same way.
3. **Attribute exclusions still apply** to everything, including explicitly included entries: hidden or system files, the size limit, folders with a marker file (`CACHEDIR.TAG`, `.nobackup`), cloud placeholders and exclusion attributes.

The ignore files themselves are backed up. Ignore files are only read inside the sources, never in the folders above a source.

**Skip files listed in .gitignore** (Standard mode, `respect_gitignore`, CLI `--respect-gitignore`) also reads `.gitignore` in every folder. In a folder that contains a `.git` folder (a repository root), it also reads `.git/info/exclude`. The `.git` folder itself is still backed up.

| Setting | Job field | CLI |
|---|---|---|
| Ignore-file names (Advanced) | `filter.ignore_files` (default `[".ufopignore"]`; empty = none) | `--ignore-file NAME` (repeatable; replaces the default; `--ignore-file ""` reads none) |
| Skip files listed in .gitignore | `filter.respect_gitignore` | `--respect-gitignore` |

## Stay on one filesystem

With **Stay on one filesystem** (Advanced, `one_file_system`, CLI `--one-file-system`), the backup does not enter a folder that is on a different filesystem from its source root. Examples are another disk mounted inside the source, a network share, `/proc` or a USB drive. The folder itself is kept as an empty folder, so a restore recreates the mount point. The run log lists each skipped folder as an *info* line: "Skipped /data/mnt: another filesystem".

The filesystem is the `st_dev` of the folder on Linux and macOS. On Windows it is the volume serial number. A symbolic link that the backup follows (symbolic links: *Follow*) counts by its target.

## Read errors: continue, fail, fail fast

Files can be unreadable (permissions, a lock, a disk error), folders can fail to list, and some entries are not files at all (sockets, named pipes, devices). **Options → Advanced → Sources and runs** decides what each of these does to the run (`error_policy`):

| Setting | Values | Default |
|---|---|---|
| Unreadable file (`on_file_error`, `--on-file-error`) | `continue`: a warning; the file keeps its copy from the previous version. `fail`: an error. | `continue` |
| Unreadable folder (`on_dir_error`, `--on-dir-error`) | `continue` / `fail` | `continue` |
| Sockets, pipes, devices (`on_unknown_entry`, `--on-unknown-entry`) | `ignore` (silently), `warn`, `fail` | `ignore` |
| Stop at the first error (`fail_fast`, `--fail-fast`) | on / off | off |
| Stop after more than N read problems (`max_errors`, `--max-errors N`) | number, or empty for never | never |

Without *fail fast*, a `fail` problem is recorded as an error and the run continues. The version is complete and the run ends with status *error* (CLI exit code 2). With *fail fast*, the run stops at the first error. The same happens when the problems (warnings and errors) exceed `max_errors`. The run then stops like a cancelled backup. The files finished so far become a partial version, and the report says why the run stopped. If the run stops while it is still scanning, nothing has been read, so no version is written. The next run starts again; data that was already uploaded is not sent twice.

The policy lives in a small shared module (`ufop_core::backup::errors`: `ErrorPolicy` and `ErrorTracker`), so restores can use it too.

## Re-check unchanged files (percentage rehash)

A file whose size and modification time did not change is normally not read again. Silent corruption, or a tool that rewrites a file and resets its timestamp, would therefore go unnoticed. **Re-check unchanged files (% per run)** (Advanced, `rehash_percent` 0–100, CLI `--rehash-percent N`) re-reads and re-hashes about N % of those files on every run.

The selection is deterministic. A hash of the file's path and a run counter (stored in the job's database) picks the files, so each run checks a different sample. With 10 %, every file is checked about once in ten runs. If a re-read file's content differs from the recorded hash although its size and time match, the run log gets this warning:

> /data/photo.raw: content changed without timestamp change (possible silent corruption); the new content was backed up

The new content is stored, and the previous versions keep the old content. The report counts `files_rehashed` and `rehash_mismatches`.

## Back up a command's output or standard input

**Back up a command's output** (Advanced, in the job's Sources step; job field `stdin_command`) runs a command at every backup. What the command prints is stored as one file in the version, for example a database dump (`pg_dump mydb`). Fields:

| Field | |
|---|---|
| `command` | Command line, split like the hook scripts'. No shell is involved unless you run one (`sh -c '…'`). |
| `path` | File name in the version. `mydb.sql` is stored as `/stdin/mydb.sql`. An absolute path such as `/dumps/mydb.sql` is used as it is. Empty means `/stdin/command-output`. |
| `timeout_secs` | Stop the command after this many seconds (0 = no limit). |

The command gets the same `UFOP_*` environment variables as the hook scripts (`UFOP_EVENTNAME=STDIN`). A non-zero exit code, a timeout or a failure to start fails the run: the run gets an error, and the message includes the end of the command's error output. The version then keeps the previous copy of the file. A job with a command needs no source folders. The virtual file is stored with mode 0644 and the time of the run. It is chunked, deduplicated and encrypted like any file, and restores like any file.

From the CLI, standard input can be backed up as a version of its own:

```bash
pg_dump mydb | ufop backup stdin --dest /mnt/backup --name mydb.sql [--note "nightly"]
```

The version holds only `/stdin/mydb.sql` (plus the `/stdin` folder). Give such backups their own destination or prefix, or their own `--job-id` in a shared repository. Otherwise the next folder backup compares itself against the stdin version. Engine API: `run_backup_with_streams(…, streams: Vec<StreamSource>)` stores any `Read` as a virtual file.

## Upload limit per run

**Upload at most per run** (Advanced, `max_upload_bytes_per_run`, CLI `--max-upload 20G`) spreads a large first backup over several runs. Once the uploaded volumes reach the limit, the run stops reading. The volume being packed, the blocks already read and the files already finished are still uploaded, so a run can go a little over the limit (about one volume). The result is a partial version marked `upload_cap_reached` in its statistics. The report gets the warning "Upload limit of … per run reached". This is a warning, not an error, and the CLI exit code is 2 because the version is partial. The next run reads the remaining files and finds the uploaded blocks again, so nothing is sent twice.

## Parallel hashing of large files

Files larger than **Hash files larger than this in parallel** (Advanced, `parallel_file_threshold`, default 256 MiB, 0 = off, CLI `--parallel-file-threshold 512M`) are spread over several threads. The chunker still runs in one pass over the file, because finding block boundaries is cheap. The block hashes, compression and encoding run on as many helper threads as *Hashing threads* allows. The blocks are put back in file order before they are stored. Block boundaries and hashes are therefore identical to sequential reading: the same file deduplicates the same way with the setting on or off. A test compares both modes byte for byte. The output of a backup command or standard input is always spread this way, because its size is not known in advance.

## ZFS and btrfs snapshots

Snapshots now also cover ZFS and btrfs on Linux, next to VSS (Windows), LVM (Linux) and APFS (macOS). **Linux snapshot method** (`snapshot.provider`, CLI `--snapshot-provider`) selects the method:

| Provider | How |
|---|---|
| `auto` (default) | By the filesystem type of each source (`findmnt`): `zfs` uses ZFS, `btrfs` uses btrfs, anything else uses LVM. Sources on different filesystems can mix methods; the report then names them, for example `zfs+lvm`. |
| `zfs` | `zfs list -H -o name,mountpoint` finds the dataset holding the source. For legacy mount points the mount itself names the dataset. The snapshot is `zfs snapshot <dataset>@ufop-<time>-<id>`, files are read from `<mountpoint>/.zfs/snapshot/ufop-…/`, and `zfs destroy` removes the snapshot afterwards. Child datasets mounted inside the source are not part of the snapshot and appear empty. |
| `btrfs` | `btrfs subvolume show` finds the subvolume holding the source (the source folder or the nearest folder above it, up to the mount point). The snapshot is `btrfs subvolume snapshot -r <subvolume> <subvolume>/.ufop-snap-<time>-<id>`, and `btrfs subvolume delete` removes it afterwards. Nested subvolumes appear empty in the snapshot. |
| `lvm` | As before: `lvcreate --snapshot`, mounted read-only, `lvremove` afterwards. |

The snapshot policy (`off`, `auto`, `on`, `required`) works as before. *Auto* falls back silently, *on* warns, and *required* fails the backup when no snapshot can be made. **Maintenance → Test snapshot** and the job diagnostics list the methods of this platform and whether their tools are installed (`providers` / `snapshot_providers`). The snapshot test runs every step and shows its output.

## Estimate before backing up

**Estimate** (Standard mode, on each job card) scans the sources like a backup and compares them with the last complete version. It does not read file contents or upload anything. It shows:

- files, folders and bytes examined; new, changed, unchanged and deleted files
- **new data**: the size of new and changed files. A changed file may share most of its blocks with its previous copy, which only reading it can tell, so with changed files this is an upper bound ("at most"). The lower bound ("at least") counts the new files plus the growth of the changed files.
- **estimated time**: new data divided by the throughput of recent successful backups (bytes uploaded ÷ duration). Without history, an assumed speed of 20 MiB/s is used; this can be changed with `assumed_bytes_per_sec`, and 0 means no time estimate.
- the 10 largest new or changed files

| Where | |
|---|---|
| App | **Estimate** on the job card |
| IPC | `backup_estimate { jobId, assumedBytesPerSec? }` |
| REST | `GET /api/v1/jobs/{id}/estimate?assumed_bytes_per_sec=…` (scope `read`) |
| CLI | `ufop backup estimate --dest … --source … [--include/--exclude/--filter "+ rclone-rule"/--ignore-file …] [--assumed-speed 20M] [--largest 10]` (no run history: the assumed speed is used) |

The output of a backup command is not included in the estimate; a warning says so.

## Back up all jobs now

**Back up all now** (Standard mode, above the job list, and in the tray menu) queues a backup of every job. Read-only jobs and jobs that are already queued or running are skipped, and the result names them with the reason. The backups go through the normal queue, so at most *Operations at the same time* (Defaults & destinations) run at once. One job never runs twice at once.

| Where | |
|---|---|
| App | **Back up all now** above the job list, and the tray menu *Back up all now* |
| IPC | `backup_run_all { filter?: { job_ids, scheduled_only, dry_run } }`, which returns `{ started: [{job_id, job_name, task_id}], skipped: [{job_id, job_name, reason}] }` |
| REST | `POST /api/v1/jobs/run-all` with the same filter (scope `run`) |
| CLI | `ufop server run-all [--job NAME …] [--scheduled-only] [--dry-run] [--wait]`, or the same as `ufop backup run-all`. The CLI has no jobs of its own, so this talks to the `ufop-server` you signed in to with `ufop server login`. With `--wait`, it waits for every started backup and exits with the worst result (0 success, 1 warning, 2 error). |

Restore options (as of a time, archives, mirror, owners, error policy, sparse files), reading single files, the volume cache and version drives are described in [backup-restore.md](backup-restore.md).
