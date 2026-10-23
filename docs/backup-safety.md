# Backup repository safety

Five safeguards protect a backup that several programs or computers can reach: repository locks, a clock check, checkpoints during long backups, read-only connections and an encrypted local database.

## Repository locks

Two programs that change one backup at the same time can damage it. A backup started on a laptop while a NAS compacts the same folder is one example. Every operation therefore takes a lock on the destination first.

| Lock | File | Taken by |
|---|---|---|
| Exclusive | `<prefix>-lock.json` | backup, compact, repair, purge, delete versions, retention, notes/tags/pins of a version, sharing a repository, the target of a version copy, rebuild missing volumes, object locks, format upgrade and rollback, reshard, password change, deleting the backup, the target of a mirror or convert |
| Shared | `<prefix>-lock-r-<id>.json` (one per reader) | restore, verify, list, browse, search, recreate database, syncing sources of a shared repository, the source of a version copy, list broken files, recovery download, the source of a mirror, dry runs |

Several readers can run together. A reader waits for a writer and a writer waits for every reader. An operation that cannot get its lock stops right away with:

> Repository in use by laptop (backup) since 2026-10-23 10:00:00 UTC

The advice in the message gives the time the lock expires.

Lock files are small unencrypted JSON documents. Each one records the owner id, host name, user, process id, program and version, operation, kind and the times the lock was taken and expires. They are not volumes. Listings, mirrors (nothing is copied or deleted), recovery downloads, database recreation, "unknown remote files" warnings, diagnostics and resharding skip them. Soft deletion removes released lock files instead of renaming them.

**Leases.** A lock lasts 30 minutes (option `lock_ttl_secs`, CLI `--lock-ttl-secs`). A background thread renews it every third of that time. If a program crashes or loses its connection, its lock expires. The next operation takes over an expired or unreadable lock and logs a warning, for example "Took over a stale repository lock held by old-pc (backup)". If someone else takes over a held lock, or breaks it, the renewal notices, the operation is cancelled and the run log says so.

**Races.** Taking a lock writes the file, reads it back, waits a moment and checks again. An exclusive lock then checks for readers, and a shared lock checks for a writer. Both sides write before they check, so when two programs start together at least one of them sees the other and backs off. A lock is released when the operation ends, fails or panics.

**Break lock.** If the program named in a lock is certainly no longer running (for example the computer was reinstalled), remove the lock by hand:

- **Maintenance → Repository safety → Break lock…** (Advanced mode, with a confirmation)
- `backup_break_lock { jobId, confirm: true, ownerId }`, or `DELETE /api/v1/jobs/{id}/lock?confirm=true&owner_id=…`
- `ufop backup lock break --dest /mnt/backup --yes [--owner-id ID]`

With an owner id only that lock is removed. If the lock changed since you looked, nothing is removed. To see the current locks, use **Maintenance → Repository safety**, `backup_lock_status`, `GET /api/v1/jobs/{id}/lock` or `ufop backup lock status` (exit code 1 while the repository is in use).

## Clock check

Leases and version times assume that this computer and the storage agree on the time. After writing its lock, the app compares the destination's modification time of the lock file with the local clock. Local disks and mounted shares, where the file server sets the time, support this, as do S3-compatible storage (`Last-Modified`) and saved connections (SFTP, FTP, WebDAV) whose listings carry file times. Other destinations skip the check.

| Difference | Result |
|---|---|
| below 5 minutes | nothing |
| 5 minutes or more (`clock_skew_warn_secs`) | a warning in the backup report and run log |
| 1 hour or more (`clock_skew_abort_secs`) | the operation stops with an error telling you to correct the time (turn on NTP) |

Set either limit to 0 to turn it off. The settings are under **Options → Advanced → Repository safety**, or use `--clock-skew-warn-secs` and `--clock-skew-abort-secs` on the CLI.

## Checkpoints

A long first backup that stops half way (a crash, a lost connection, a laptop that is shut down) would otherwise leave nothing restorable. During a backup, each time a volume upload completes and the interval has passed (`checkpoint_interval_secs`, 15 minutes by default), the engine uploads a checkpoint. A checkpoint is a sealed partial file list of every file whose blocks are all uploaded. It can also be triggered by data volume (`checkpoint_interval_bytes`, off by default). Each new checkpoint replaces the run's previous one, which is deleted after the new one is stored. When the run finishes, the complete version replaces the checkpoint. A cancelled run writes its usual partial version and also removes the checkpoint.

A checkpoint is an ordinary partial version. You can restore and browse it, retention counts it like any other version, and the next backup does not compare files against it. The next backup reads the files again but finds their blocks already uploaded, so only the rest is sent. Set both intervals to 0 to turn checkpoints off. The settings are under **Options → Advanced → Repository safety**, or use `--checkpoint-interval-secs` and `--checkpoint-interval-bytes`. The backup report counts them in `checkpoints`.

## Read-only connections

A job marked **Read-only (restore and browse only)** (job editor → Destination, `read_only: true`, **Continue this backup on this computer → Read-only**, `adopt_repository(readOnly)`, CLI `--read-only`) never writes to its destination:

- The connection itself refuses uploads, deletes, renames, folder creation and object locks, with a "connected read-only" error.
- The app refuses writing operations before they start: backup, compact, repair, purge, delete versions, rebuild, object locks, format upgrade, reshard, password change, deleting the backup and breaking a lock. Scheduled backups never run, and the job list shows a **Read-only** badge.
- Restore, browse, search, verify (which never repairs), dry runs, recovery downloads and database recreation work.

**Read locks on read-only connections are best effort.** No lock file can be written, so a read-only job only checks the existing locks. It does not start while a live exclusive lock exists, but other programs cannot see it. Writers that start later do not wait for it. Use read-only connections for archives and for other people's backups, where nobody else writes at the same time. `Backend::is_read_only()` reports the mode, so destinations that request access scopes can ask for read-only credentials.

## Encrypted local database

Each job keeps a local cache (`<data dir>/<job id>.sqlite`) of the repository: volumes, blocks and every version's file list. For an encrypted backup that cache would otherwise show your file names to anyone who can read the disk.

**How it is encrypted.** The columns that describe your files are sealed with AES-256-GCM, each value with its own random nonce and bound to its column as associated data:

- paths and symbolic-link targets
- extended metadata
- file hashes and block lists
- every settings value, including the embedded job configuration and change-journal state

A key check stored in the database tells whether a key is the right one. Volume names, sizes and version times stay readable, because the destination shows them anyway. So does the block-hash index the deduplicator looks up on every block. Database files are created with owner-only permissions on Unix. Encrypting a database, changing its key or decrypting it happens in one transaction, and the file is then rewritten (`VACUUM`, WAL truncated) so no plaintext stays behind.

*Why not SQLCipher:* bundling SQLCipher needs OpenSSL at build time on Windows and macOS, and the vendored build was not available. It would also replace the SQLite library of the whole app. Column encryption works on every platform with the crypto the engine already uses.

**Key.** A random 256-bit key per job, kept in the OS keychain (or in the app's sealed settings store on machines without one), like the backup passphrase. It never leaves this computer and never goes to the destination.

**When it is on.**

- New encrypted jobs encrypt their database from the start (`encrypt_local_database`).
- Jobs created before this feature keep a plain database until you choose **Maintenance → Repository safety → Encrypt local database** (`backup_set_database_encryption`, `PUT /api/v1/jobs/{id}/database-encryption`). The job editor has the same switch. Turning it off decrypts the database again.
- CLI: `--db-key-source keyring` (OS keychain, key created on first use), `--db-key-source env:VAR` (64 hex digits) or `--db-key-source none` (default, plain). `ufop backup db encrypt <db> [--decrypt]` converts a database in place, and `ufop backup db sanitize` takes `--db-key-source` too.

**Losing the key** does not lose the backup. When the database is encrypted and its key is missing or different, the app sets the file aside as `<name>.undecryptable-<time>`, starts a new database and rebuilds it from the destination (Recreate database). The run log says so, and the next backup continues normally.
