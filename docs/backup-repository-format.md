# Backup repository format (v1 and v2)

Every backup destination holds a header, `<prefix>-repo.json`, that records how the repository is written. The app reads and writes **format 1** and **format 2**. New backups use format 2 unless you pick format 1. A format 1 backup stays format 1 until you upgrade it.

## What format 2 adds

| | Format 1 | Format 2 |
|---|---|---|
| Blocks | Fixed size (`block_size`) | Content-defined (FastCDC) by default: average `block_size`, minimum ¼, maximum 4×. Fixed size is still available. |
| Block compression | Deflate | Zstandard (pack flag 2) by default; Deflate is still available. Blocks that do not shrink are stored. |
| File lists and indexes (`dlist`, `dindex`) | JSON, sealed as is | JSON compressed with Zstandard, then sealed |
| Encryption key | The Argon2id key encrypts volumes | A random 256-bit master key encrypts volumes. The header stores it wrapped by the Argon2id key, so changing the password rewrites only the header. |
| Cipher | AES-256-GCM | AES-256-GCM or ChaCha20-Poly1305 |
| Layout | Flat | Flat, or sharded: `dblock` and `dindex` volumes in subfolders named after the first 1–4 hex digits of the SHA-256 of the volume name |

The header lists every format 2 feature in `features` (`envelope_key`, `zstd_blocks`, `zstd_metadata`, `content_defined_chunking`, `chacha20_poly1305`, `sharded_layout`). A version that finds a feature it does not know refuses the repository rather than misreading it. Readers accept every block flag and both ciphers in any repository, so a format 2 repository can hold format 1 volumes.

Content-defined chunking finds the same blocks again after an insertion or deletion inside a file, so editing the start of a large file uploads a few new blocks instead of the whole file. The chunker is fixed once the repository exists: changing it would stop new blocks from matching old ones.

### Choosing options

Set these when you create the job, under **Options → Advanced → Repository format**, or with `ufop backup run`:

| Option | CLI flag | Values |
|---|---|---|
| Format | `--format-version` | `2` (default), `1` |
| Chunking | `--chunker` | `fastcdc` (default on format 2), `fixed` |
| Block compression | `--block-codec` | `zstd` (default on format 2), `deflate` |
| Cipher | `--cipher` | `aes256-gcm` (default), `chacha20-poly1305` |
| Layout | `--layout`, `--shard-prefix-len` | `flat` (default), `sharded` with 1–4 digits |

Format 1 refuses the format 2 options. The sharded layout needs a destination with folders (local disks and mounted shares). Cloud object stores keep the flat layout.

## Upgrade and rollback

**Maintenance → Repository format → Upgrade to format 2**, `{"type": "upgrade_format"}` through `backup_execute` or `POST /api/v1/jobs/{id}/operations`, or:

```bash
ufop backup upgrade --dest /mnt/backup
```

Upgrading rewrites only the header:

1. The current header is saved as `<prefix>-repo.v1.bak.json`.
2. The new header is written. For a built-in encrypted backup the master key is the existing Argon2id key, wrapped by itself, so the same passphrase opens the backup and every existing volume stays readable.
3. New data uses Zstandard blocks and compressed file lists. Chunking stays fixed-size, so new blocks keep matching existing ones.

The new header carries `v1_rollback_available` until the first write in format 2 (a backup, compaction, repair or file-list rewrite). Until then you can roll back:

```bash
ufop backup upgrade --dest /mnt/backup --rollback
```

Rollback restores the saved format 1 header and deletes the copy. It is refused once format 2 data was written, after a password change, and for repositories created as format 2. The error says so; keep using format 2 in that case (it reads everything). **Recovery → Convert** can write a separate copy in any format.

## Changing the password

Format 2 backups with built-in encryption: **Edit job → General → Change password**, `backup_change_passphrase`, `PUT /api/v1/jobs/{id}/passphrase`, or:

```bash
UFOP_PASSPHRASE=old UFOP_NEW_PASSPHRASE=new ufop backup change-password --dest /mnt/backup [--kdf-preset sensitive]
```

The master key is unwrapped with the old passphrase and wrapped again with a key derived from the new one (new salt, optionally a new Argon2id cost). The app keeps the previous header as `<prefix>-repo.bak.json`, writes the new header to `<prefix>-repo.new.json`, moves it into place, reads it back and only then deletes the copy. If something fails, the old header still works (or is in the `.bak.json` copy). The app stores the new passphrase for the job.

Format 1 backups need an upgrade first.

## Resharding

**Maintenance → Repository format → Move volumes to**, `{"type": "reshard", "layout": {"type": "sharded", "prefix_len": 2}, "dry_run": false}`, or:

```bash
ufop backup reshard --dest /mnt/backup --layout sharded --shard-prefix-len 2 [--dry-run]
ufop backup reshard --dest /mnt/backup --layout flat
```

Each volume is copied, read back, compared and only then deleted from its old place. The header changes after every volume has moved. An interrupted run leaves every volume readable (the engine looks in both places) and running it again finishes the move. Mirror, recovery download, database recreation, diagnostics and job deletion all follow the layout.

Locks, the clock check, checkpoints, read-only connections and the encrypted local database are described in [backup-safety.md](backup-safety.md). Lock files (`<prefix>-lock.json`, `<prefix>-lock-r-<id>.json`) always stay at the top of the destination, in every layout.

## Version metadata

Every file list (`<prefix>-<time>.dlist`) can carry these optional fields next to `timestamp`, `partial` and `files`. They are written only when set:

| Field | Meaning |
|---|---|
| `note` | Free text, up to 2000 characters |
| `tags` | `{"key": "value"}`; keys 1–64 of `A-Z a-z 0-9 _ . : -`, values up to 256 characters, at most 32 tags |
| `pinned`, `pin_reason` | "Keep forever": retention and purge never remove or rewrite the version, and deleting it needs `force` |
| `identity` | `{hostname, username, job_id, source_label}` of the client that wrote the version |
| `stats` | `{bytes_added, new_blocks, files_added, files_changed, files_deleted}` recorded at backup time. `bytes_added` counts the sealed block volumes and their indexes uploaded for the version, after compression and encryption. The file list itself is not counted. |

The fields work in format 1 and format 2. File-list readers have never refused unknown fields, so older app versions read these lists and ignore the metadata. If an older version rewrites a file list (purge or repair), it drops the metadata. Editing a note, tags or a pin downloads the version's file list and checks it against its recorded hash. The list is then sealed again with the change and uploaded under the same name, which replaces the old object in one write. A database rebuild reads the metadata back. The local database keeps a copy in `filesets.meta` (schema 3), encrypted like file paths when the database is encrypted. Bug-report copies keep only the pin and the statistics.

## Shared repositories and client identity

A format 2 repository can be **shared**: several jobs, on one computer or on many, back up into the same destination and prefix. Data they have in common is stored once. The header then lists the feature `shared_repository`. App versions that do not know the feature refuse the repository instead of misreading it.

- **Create** a shared repository with the job option *Share a new repository with other computers and jobs* (`shared_repository`, CLI `--shared`). **Share an existing one** with *Maintenance → Sharing → Share this repository with other computers/jobs*, the `enable_sharing` operation, or `ufop backup share`. The existing versions become the enabling job's own (`--keep-identities` keeps versions that already name another identity). The file lists are rewritten first and the header last. Format 1 repositories need an upgrade first.
- **Sources.** Each version records the identity that wrote it. Its *source id* is a hash of `user@host:job` (the host name is compared without case). Listing, restore, retention, pins, purge and comparisons default to the job's own source. Another source can be named explicitly (`source` in the API, `--source-id` in the CLI, the source picker in Restore, `*` for all). `ufop backup sources`, `GET /jobs/{id}/sources` and *Maintenance → Sharing* list the sources. Retention never touches another source unless it is targeted.
- **Identity override.** A job can record another host or user name (*Options → Advanced → Versions and sharing*, CLI `--hostname`/`--username`, `--job-id` for CLI jobs). This lets a new computer continue the old computer's history. Adopting a shared repository with *Continue this backup on this computer* always creates a new source, so the new computer does not take over another job's history.
- **Deduplication across clients.** Before every operation the local database is brought up to date from the destination. Index volumes it has not seen add the blocks other clients uploaded, so identical data is not uploaded again. File lists it has not seen, or whose size changed, add or update other sources' versions. Volumes that disappeared (another client compacted them) are forgotten. The names of known volumes live in the database, so an unchanged repository costs one listing. Version times are unique across sources because file-list names are made from them.
- **Writers** are serialised by the exclusive repository lock (see [backup-safety.md](backup-safety.md)). Each writer syncs while it holds the lock.
- **Compaction** counts the blocks of every source's versions as used. It also leaves volumes younger than `shared_gc_delay_hours` (default 4) alone. The age counts from the later of when this computer first saw the volume and the destination's modification time. A writer that crashed after uploading blocks but before its file list therefore loses nothing.

## Copying versions to another repository

*Restore → Copy to another backup…* (Advanced), the `copy_versions` operation (target: another job or a destination with a passphrase, prefix and options for a new repository) or `ufop backup copy-versions` copy chosen versions into another repository. Only the blocks the versions use and the target lacks are transferred. Each source volume is downloaded once. Blocks are decoded and encoded again for the target's codec, cipher and volume format. When the target cuts or hashes blocks differently, for example from format 1 fixed blocks into format 2 content-defined chunking, the file contents are re-assembled and cut again. Identical contents are processed only once. Each copied version gets a new file list in the target's format with its time, note, tags, pin and identity. Its statistics describe what the copy added to the target. A version already in the target (same time and files) is skipped. A clashing time moves by one second, and the report says so. The copy takes a shared lock on the source and an exclusive lock on the target. A dry run counts blocks and bytes and writes nothing.

## Format details

`ufop backup format-info --dest …` and the `format_info` operation show the version, chunker, codecs, cipher, layout and features. The independent recovery script (`scripts/ufop-recover.py`) reads both formats. Zstandard data needs Python 3.14+ or the `zstandard` package.
