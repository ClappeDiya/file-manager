# Restoring and reading backups

This page covers what you can do with a backup version besides an ordinary restore:

- restore as it was at a date and time
- restore into a ZIP or TAR archive
- mirror restore, which also removes extra files
- restoring file owners
- the restore error policy
- sparse files
- previewing and reading one file without restoring it
- the local volume cache
- opening a version as a read-only drive

Versions, notes and pins are in [backup-versions.md](backup-versions.md). Locks are in [backup-safety.md](backup-safety.md). Every feature here reads the repository under a **shared** lock, so several of them can run together. Backups wait for them to finish.

## Restore as it was at a time

**Restore → Restore as it was at** (Standard mode) takes a date and time in your local time zone. The app picks the newest **complete** version made at or before that time and shows which one ("Uses the version from 20 Oct 2026, 12:00"). If no complete version is that old, it says so. The restore then names the time instead of a version number, so a backup that finishes in the meantime does not change which version is used. Interrupted (partial) versions and checkpoints are skipped. In Advanced mode, *Allow interrupted (partial) versions* lets the time pick them too.

| Where | |
|---|---|
| Engine | `RestoreOptions.selector`: `{"type": "as_of", "time": "2026-10-01T09:00:00Z", "allow_partial": false}`, or `{"type": "latest"}`, `{"type": "oldest"}`, `{"type": "index", "index": 2}`, `{"type": "id", "id": "<timestamp or file-list name>"}`. When no selector is given, `version` (0 = newest) still applies. |
| CLI | `ufop backup restore … --time "2026-10-01 09:00"` (local time), `--time 2026-10-01` (the end of that day), `--time 2026-10-01T09:00:00Z` (RFC 3339), `--version latest\|oldest\|N\|<timestamp>`, `--allow-partial` |
| REST | `?time=…`, `?version=latest\|oldest\|N\|<id>` and `?allow_partial=true` on `GET /jobs/{id}/files` and `GET /jobs/{id}/file`. `GET /jobs/{id}/versions/resolve` returns `{index, fileset}` for a time or version. Restores use `selector` in the `restore` operation. |
| IPC | `backup_resolve_version { id, selector, source? }` |

If two versions have exactly the same time, as can happen when several sources of a shared repository are listed together, the first one listed (the newer entry) is used. A version made exactly at the given time counts as "at or before".

## Restore into an archive (ZIP, TAR, TAR.GZ)

**Restore → Download as ZIP…** (Standard) asks where to save the archive. It writes the selected files, or the whole version, into one archive. Nothing is restored to disk. The file name decides the type: `.zip`, `.tar`, `.tar.gz` or `.tgz`.

Files are streamed into the archive block by block. The app keeps only a few unpacked volumes in memory, never a staging copy of the files. The archive is written as `<name>.partial` and renamed when it is complete. A failed or cancelled export leaves nothing behind. Each file's content is checked against its recorded hash. A file whose data is missing at the destination is left out and reported as an error.

| | TAR / TAR.GZ | ZIP |
|---|---|---|
| Folders | yes | yes |
| Files | yes | yes, ZIP64 when large |
| Symbolic links | yes | as Unix link entries (Info-ZIP style) |
| Permissions (mode) | yes | yes (Unix attributes) |
| Modification times | yes, to the second | yes, 2-second precision, local time, years 1980–2107 |
| Owner (uid/gid) | yes, when recorded | no |
| Windows alternate data streams | left out, with a warning | left out, with a warning |

Paths inside the archive are relative to the deepest folder holding the selection, as in a restore to another folder. *Keep the full original folder structure* keeps the full paths.

```bash
ufop backup restore --dest /mnt/backup --archive ~/docs.zip [--time "2026-10-01 09:00"] [/home/me/Docs]
ufop backup restore --dest /mnt/backup --archive out.bin --archive-format tgz    # zip | zip-stored | tar | tgz
```

Engine and API: `RestoreOptions.archive = {"path": "/tmp/out.zip", "format": null | "zip" | "zip_stored" | "tar" | "tar_gz"}`.

## Mirror restore

**Restore → Mirror: delete files in the target folder that are not in this version** makes the restored folders exactly match the version. It deletes anything else in them. It needs a target folder and works in three steps:

1. The app runs the restore as a dry run and lists what would be deleted ("12 files and 1 folder will be deleted from the target folder").
2. You confirm the list. The Safety Interlock assesses the deletion like any large delete, and the backend issues a single-use confirmation token.
3. The restore runs. The deletions happen only after every file was restored and only if the restore was not stopped early.

Rules that keep it safe:

- Only folders that the restore itself recreates are mirrored. Restoring a single file never deletes its neighbours.
- Every deletion is inside the target folder. The target folder itself is never deleted.
- Symbolic links are deleted as links. Nothing is followed.
- On Windows and macOS, names are compared without case, so a file whose name differs only in case is kept.

| Where | |
|---|---|
| Engine | `RestoreOptions.delete_extraneous = true` (needs `target`). A dry run lists the paths in `planned_deletions` (folders end in `/`). The report counts `files_deleted` and `folders_deleted`. |
| Manager / REST | A mirror restore that is not a dry run is refused unless `overrides.confirm_delete` is `true`: `{"type": "restore", "options": {…, "delete_extraneous": true}, "overrides": {"confirm_delete": true}}` |
| IPC | `backup_execute` and `backup_start_operation` take `confirmationToken`. With `confirm_delete`, the token must come from `request_destructive_confirmation { operation: "backup_mirror_restore" }`. It is single use and expires after 30 seconds. |
| CLI | `ufop backup restore … --target ~/Docs --mirror` lists the deletions and asks. `--yes` skips the question. Without a terminal and without `--yes` it refuses. `--dry-run` only lists. |

## File owners

New backups record each entry's owner and group id on Linux and macOS (`uid`, `gid` in the entry metadata). On Windows the owner is the `O:` part of the recorded security descriptor, which is captured with *Back up ACLs*. Versions made before this have no owner and restore as before.

| `restore_owner` | |
|---|---|
| `auto` (default) | Owners are restored when the restore runs with the privileges to do so: root or `CAP_CHOWN` on Linux, root on macOS, `SeRestorePrivilege` on Windows (an administrator; the app enables the privilege). Without them owners are skipped, and one warning says how many items were affected. |
| `always` | As `auto`, but the restore refuses to start without the privileges. |
| `never` | Files belong to whoever runs the restore. |

The owner is set before the permissions, because changing the owner clears set-user-id bits. Links get their own owner (`lchown`). CLI: `--restore-owner auto|always|never`. App: **Restore → Owners, errors and sparse files** (Advanced).

## Restore error policy

A restore uses the same policy machinery as a backup (`ufop_core::backup::errors`, `RestoreErrorPolicy`):

| Problem | Option | CLI | Default |
|---|---|---|---|
| A file or link cannot be written | `on_write_error` | `--on-file-error fail\|continue` | `fail` (an error; the rest is still restored) |
| A folder cannot be created | `on_dir_error` | `--on-dir-error` | `fail` |
| Data is missing or damaged at the destination | `on_missing_data` | `--on-missing-data` | `fail` |
| Permissions, owner, times or attributes cannot be applied | `on_metadata_error` | `--on-metadata-error ignore\|warn\|fail` | `warn` |
| Stop at the first error | `fail_fast` | `--fail-fast` | off |
| Stop after more than N problems | `max_errors` | `--max-errors N` | never |

`continue` turns that kind of problem into a warning. When the restore stops early, `stop_reason` in the report says why, and a mirror restore skips its deletions. The defaults match how restores always behaved.

## Sparse files

Virtual machine disks, databases and similar files often contain long runs of zeros that the filesystem stores as holes. With `sparse: auto` (the default), the restore checks each 4 KiB piece aligned to the file offset. A piece that is all zeros is skipped with a seek instead of being written, and the file's final length is set at the end. The file gets its holes back instead of filling the disk. The content is byte for byte the same, and the file still goes through the temporary-file-and-rename step and the hash check. On Windows the file is first marked sparse (`FSCTL_SET_SPARSE`). Filesystems without holes simply store the zeros. `sparse: off` (CLI `--sparse off`) writes every byte. The report counts `sparse_files`. Backups do not record whether a file was sparse. Any long zero run becomes a hole.

## Read or preview one file

The file list in **Restore** has a **Preview** link next to each file (Standard). The app copies that one file from the selected version into a temporary file in its data folder. It opens the copy in the app's preview pane: text, code, images, PDF, office documents and more. *Save a copy…* exports the file to a place you choose. Preview copies older than an hour are removed, and all of them when the app exits. Files over 512 MiB are too large to preview. Export them instead.

Only the volumes that hold the file are downloaded, through the volume cache. A range read downloads only the volumes holding that range.

| Where | |
|---|---|
| Engine | `access::BlockSource` (block → volume lookup, lazy download, bounded unpacked-volume memory) and `access::open_file(&source, &entry)`, which returns a `Read + Seek` reader with `read_range(offset, len)` |
| IPC | `backup_read_file { id, path, version?, selector?, offset?, length?, source? }` returns base64 data (at most 10 MiB per call), with `size` and `truncated`. `backup_export_file { …, dest }` copies the file. `backup_preview_file` makes the temporary copy for the preview pane. |
| REST | `GET /jobs/{id}/file?path=…&version=…&time=…` streams the file. A single `Range` header gets `206` with `Content-Range`, and a range past the end gets `416`. |
| CLI | `ufop backup cat --dest … <path> [--version latest\|N] [--time …] [--offset N] [--length N] > out` |

## The local volume cache

Previews, drives, `cat` and repeated reads keep downloaded volumes in a local cache. The volumes are stored exactly as the destination holds them, still encrypted for encrypted backups, so the cache is as safe at rest as the destination.

- Every entry is checked against the volume's recorded SHA-256 and size on each read. A damaged or truncated entry is deleted and downloaded again. Volumes without a recorded hash never go through the cache.
- The least recently used entries are removed once the total passes the limit (default 2 GiB). A cache hit refreshes the entry's file time, so the order survives restarts and is shared by every program that uses the folder.
- Entries are written to a temporary file and renamed into place, so other threads and processes never see half an entry. Each repository (destination and prefix) has its own subfolder.
- Full restores do not use the cache unless you turn it on (*Also use the cache for full restores*, `use_for_restore`, per restore `use_cache` or `--use-cache`). A large restore reads each volume once and would only push everything else out. Verification never reads from the cache, so it always checks the destination, but it keeps what it downloaded.

| Where | |
|---|---|
| App | **Settings → Volume cache**: usage and *Clear cache* (Standard). Folder, size limit (0 = off) and use for full restores (Advanced). |
| IPC | `backup_cache_stats`, `backup_clear_cache`, `backup_get_cache_settings`, `backup_set_cache_settings { settings: {dir, max_bytes, use_for_restore} }` |
| REST | `GET /backup-cache` (stats: entries, bytes, hits, misses, corrupt, evictions, hit rate), `DELETE /backup-cache`, `GET`/`PUT /backup-cache/settings` |
| CLI | `ufop backup cache stats\|clear [--cache-dir DIR] [--cache-size 2G]`. `cat`, `mount` and `restore --use-cache` take the same flags. |

Hit and miss counts are kept from program start.

## Open a version as a drive

**Restore → Open as drive** (Standard) serves the selected version as a read-only WebDAV drive. *All versions as drive* serves every version, one folder per version named by its local date and time (`2026-10-20 12-00-00`). The app then mounts the drive with your system's own WebDAV client:

| System | How | Where the files appear |
|---|---|---|
| Linux | `gio mount dav://…` (GVfs, used by GNOME and most file managers) | `/run/user/<uid>/gvfs/dav:host=127.0.0.1,port=…` and the file manager's sidebar |
| macOS | `mount_webdav` | a folder in the app's data folder |
| Windows | `net use * http://…` (the WebClient service) | the next free drive letter |

If the tool is missing or mounting fails, **Open drives** shows the address and the reason. Connect to the address in your file manager instead (*Connect to Server*, *Map network drive*, or `dav://…`). Clients that ask for a login take any user name and the password shown. **Unmount** closes a drive. Drives also close when their job is deleted and when the app exits. With an idle timeout (`idle_timeout_minutes`), a drive closes after that long without a request.

How it works:

- The server listens on `127.0.0.1` on a random free port. Access needs a random 128-bit token, either as the first part of the path (`http://127.0.0.1:PORT/<token>/`, which mount tools use without a password prompt) or as the password of HTTP Basic authentication, or `Authorization: Bearer <token>`.
- It answers `OPTIONS`, `PROPFIND` (depth 0 and 1) and `GET`/`HEAD` with single byte ranges. Every method that would change something (`PUT`, `DELETE`, `MKCOL`, `MOVE`, `COPY`, `PROPPATCH`, `LOCK`, …) gets `405 Method Not Allowed`. Symbolic links are not shown.
- File data comes through the file reader and the volume cache. Opening a file downloads only the volumes it needs.
- The repository's shared lock is held while the drive is set up, not while it is open, so scheduled backups keep running. A compaction while a drive is open can make files of removed versions unreadable. Unmount and open the drive again.
- The server code (`ufop_core::backup::davserve`) shares its WebDAV building blocks (`ufop_core::webdav`: ranges, `PROPFIND` XML, content types) with the server's `/dav` share.

| Where | |
|---|---|
| IPC | `backup_open_drive { id, request: {version?, selector?, all_versions, source?, mount?, port?, idle_timeout_minutes?} }` returns `{id, url, username, password, mount_point, mounted_with, mount_error, …}`. `backup_close_drive { driveId }` and `backup_list_drives` |
| CLI | `ufop backup mount --dest … [--version latest\|N] [--time …] [--all] [--bind 127.0.0.1] [--port 0]` serves until Ctrl-C and prints the address and password. Mount it with the commands above. |

Drives are a desktop feature. The headless server has no drive endpoints, because its `127.0.0.1` is not your computer's. Use the server's `/dav` share or `GET /jobs/{id}/file` instead.
