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

## Format details

`ufop backup format-info --dest …` and the `format_info` operation show the version, chunker, codecs, cipher, layout and features. The independent recovery script (`scripts/ufop-recover.py`) reads both formats. Zstandard data needs Python 3.14+ or the `zstandard` package.
