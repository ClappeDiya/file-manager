# UFOP CLI Reference

Version 0.1.0

---

## Overview

The `ufop` command-line interface provides scriptable access to the Unified File Operations Platform. It is written in Rust for performance and cross-platform support.

### Installation

The CLI is distributed as a standalone binary. It requires Rust 1.77 or later to build from source.

```bash
# Build from source
cd cli
cargo build --release
```

### Global Options

| Flag | Description |
|------|-------------|
| `--format <FORMAT>` | Output format: `human`, `json`, `yaml`, `csv` (default: the profile's `default_format`, else `human`) |
| `--json` | Shorthand for `--format json` |
| `--yaml` | Shorthand for `--format yaml` |
| `--csv` | Shorthand for `--format csv` |
| `--profile <NAME>` | Named profile to use for this run (or set `UFOP_PROFILE`); see `ufop profile` |
| `--dry-run` | Show what would happen without executing |
| `--limit <RATE>` | Bandwidth limit (e.g., `1M`, `500K`, `2G`) |
| `--api-url <URL>` | API server URL for this run (or set `UFOP_API_URL`); not saved unless a command changes it |
| `-v`, `-vv`, `-vvv` | Verbosity level (info, debug, trace) |

### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Partial failure |
| 2 | Full failure |
| 3 | Authentication error |
| 4 | Policy violation |

### CSV output

`--csv` writes RFC 4180 records (CRLF line ends) to stdout; status messages go to stderr, so `ufop … --csv > out.csv` is pure data. Lists become one row per item (columns in first-seen order), a single result one row, and nested values compact JSON. Cells a spreadsheet would treat as a formula (starting `=`, `+`, `-`, `@`, other than numbers) are prefixed with `'`. In `--json` mode `ufop transfer` also streams progress events, one JSON object per line; YAML and CSV print only the result.

### Configuration and profiles

Settings live in `~/.config/ufop/config.toml` (owner-only; `UFOP_CONFIG` overrides the path). Named profiles keep separate API servers, tokens, `ufop server` sign-ins and default output formats; saved connections are shared.

```bash
ufop profile create staging --api https://staging.example.com --default-format json
ufop --profile staging login --token <TOKEN>      # signs in to that profile only
ufop --profile lab server login --url http://lab:8200 --password …
ufop profile use staging                           # default for later runs
ufop profile list                                  # `*` marks the active profile
ufop profile show                                  # tokens are masked
ufop --profile default status                      # the top-level settings
ufop profile remove staging                        # not while it is in use
```

The active profile is `--profile` / `UFOP_PROFILE`, else the one chosen with `ufop profile use`, else `default`. A `--profile` that does not exist is an error, except for `login` and `server login`, which create it. `ufop profile create --from <name>` copies another profile, tokens included.

---

## Commands

### `ufop login`

Authenticate with the UFOP server.

```bash
# Interactive login
ufop login

# Login with API token
ufop login --token <TOKEN>

# Login to specific server
ufop login --server https://api.example.com --token <TOKEN>
```

**Options:**

| Flag | Description |
|------|-------------|
| `--token <TOKEN>` | API key or authentication token |
| `--server <URL>` | Server URL to authenticate against |

---

### `ufop connection`

Manage saved connections (SFTP, FTP, WebDAV, S3, etc.).

#### `ufop connection list`

List all saved connections.

```bash
# List all connections
ufop connection list

# Filter by protocol
ufop connection list --protocol sftp

# Output as JSON
ufop connection list --json
```

**Options:**

| Flag | Description |
|------|-------------|
| `--protocol <PROTO>` | Filter by protocol (sftp, ftp, webdav, s3, etc.) |

#### `ufop connection add`

Add a new connection.

```bash
# Add an SFTP connection
ufop connection add my-server sftp://user@host.example.com

# Save password in keychain
ufop connection add prod-server sftp://deploy@prod.example.com --save-password

# Add an S3 connection
ufop connection add my-bucket s3://my-bucket.s3.amazonaws.com
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `<NAME>` | Connection name |
| `<URI>` | Connection URI (e.g., `sftp://user@host`, `ftp://host`, `s3://bucket`) |

**Options:**

| Flag | Description |
|------|-------------|
| `--save-password` | Save password in the OS keychain |

#### `ufop connection test`

Test connectivity of a saved connection or URI.

```bash
ufop connection test my-server
ufop connection test sftp://user@host.example.com
```

#### `ufop connection remove`

Remove a saved connection.

```bash
ufop connection remove my-server
```

---

### `ufop transfer`

Transfer files between locations.

```bash
# Local to local
ufop transfer /path/to/source /path/to/dest

# Local to remote
ufop transfer ./data sftp://server/backup/data

# Remote to local
ufop transfer s3://my-bucket/archive.zip ./downloads/

# With options
ufop transfer /src /dst --overwrite --verify --parallel 8

# Dry run
ufop transfer /src /dst --dry-run

# With bandwidth limit
ufop transfer /large-file s3://bucket/file --limit 10M

# Resume interrupted transfer
ufop transfer /src /dst --resume
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `<SRC>` | Source path or URI |
| `<DEST>` | Destination path or URI |

**Options:**

| Flag | Description |
|------|-------------|
| `--overwrite` | Overwrite existing files at destination |
| `--resume` | Resume an interrupted transfer |
| `--verify` | Verify checksums after transfer |
| `--parallel <N>` | Number of parallel streams (default: 4) |

---

### `ufop sync`

Manage sync pairs.

#### `ufop sync list`

List all sync pairs.

```bash
ufop sync list
ufop sync list --json
```

#### `ufop sync create`

Create a new sync pair.

```bash
# One-way sync
ufop sync create my-backup /home/user/documents /mnt/backup/documents

# Bidirectional sync
ufop sync create team-files /shared/docs sftp://nas/docs --direction bidirectional

# Scheduled sync
ufop sync create nightly-backup /data /backup/data --schedule "0 2 * * *"
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `<NAME>` | Sync pair name |
| `<SOURCE>` | Source path or URI |
| `<DEST>` | Destination path or URI |

**Options:**

| Flag | Description |
|------|-------------|
| `--direction <DIR>` | `bidirectional`, `source-to-dest` (default), `dest-to-source` |
| `--schedule <CRON>` | Cron expression for automatic sync |

#### `ufop sync run`

Run a sync pair.

```bash
ufop sync run my-backup
ufop sync run my-backup --dry-run
```

#### `ufop sync delete`

Delete a sync pair.

```bash
ufop sync delete my-backup
```

---

### `ufop compat`

Check file name compatibility across platforms.

```bash
# Check specific paths
ufop compat /path/to/check

# Check against specific platforms
ufop compat /path/to/check --targets windows,linux

# JSON output
ufop compat /path/to/check --json
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `<PATHS>` | One or more paths to check |

**Options:**

| Flag | Description |
|------|-------------|
| `--targets <PLATFORMS>` | Comma-separated list: `windows`, `macos`, `linux` |

---

### `ufop checksum`

Compute or verify file checksums.

```bash
# Compute SHA-256 (default)
ufop checksum file.txt

# Compute MD5
ufop checksum file.txt --algorithm md5

# Multiple files
ufop checksum file1.txt file2.txt file3.txt

# Verify against known hash
ufop checksum file.txt --verify abc123def456...

# JSON output
ufop checksum file.txt --json
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `<PATHS>` | One or more files to checksum |

**Options:**

| Flag | Description |
|------|-------------|
| `--algorithm <ALG>` | `md5`, `sha1`, `sha256` (default: `sha256`) |
| `--verify <HASH>` | Verify file matches the provided hash |

---

### `ufop duplicates`

Find duplicate files in a directory.

```bash
# Report duplicates
ufop duplicates /path/to/scan

# Only consider files above 1MB
ufop duplicates /path/to/scan --min-size 1048576

# Auto-delete, keeping newest
ufop duplicates /path/to/scan --action keep-newest

# Dry run before deleting
ufop duplicates /path/to/scan --action keep-newest --dry-run
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `<PATH>` | Directory to scan |

**Options:**

| Flag | Description |
|------|-------------|
| `--min-size <BYTES>` | Minimum file size to consider (default: 1) |
| `--action <ACTION>` | `report` (default), `keep-newest`, `keep-largest`, `delete` |

---

### `ufop rename`

Batch rename files using patterns or find/replace.

```bash
# Pattern-based rename
ufop rename /path/to/files --pattern "vacation_{counter}.{ext}"

# Find and replace
ufop rename /path/to/files --find "old_prefix" --replace "new_prefix"

# Regex find and replace
ufop rename /path/to/files --find "IMG_(\d+)" --replace "photo_\1" --regex

# Case transform
ufop rename /path/to/files --case lower

# Counter with padding
ufop rename /path/to/files --pattern "{name}_{counter}.{ext}" --start 1 --step 1 --pad 3

# Preview changes without applying
ufop rename /path/to/files --find "old" --replace "new" --dry-run
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `<PATH>` | File or directory to rename |

**Options:**

| Flag | Description |
|------|-------------|
| `--pattern <PATTERN>` | Pattern with tokens: `{name}`, `{ext}`, `{num}`, `{date}`, `{parent}`, `{counter}` |
| `--find <STRING>` | String to find (literal or regex) |
| `--replace <STRING>` | Replacement string |
| `--regex` | Use regex for find/replace |
| `--case <CASE>` | Case transform: `upper`, `lower`, `title`, `sentence` |
| `--start <N>` | Counter start value (default: 1) |
| `--step <N>` | Counter step (default: 1) |
| `--pad <N>` | Zero-padding width for counter (default: 0) |

---

### `ufop archive`

Archive operations (create, extract, list).

#### `ufop archive create`

Create an archive.

```bash
# Create a ZIP
ufop archive create output.zip file1.txt dir1/

# Create a tar.gz
ufop archive create backup.tar.gz /data --format tar.gz

# Encrypted ZIP
ufop archive create secure.zip secret.txt --password mypassword

# Custom compression level
ufop archive create output.zip data/ --level 9
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `<OUTPUT>` | Output archive path |
| `<PATHS>` | Files and directories to include |

**Options:**

| Flag | Description |
|------|-------------|
| `--format <FMT>` | `zip` (default), `tar`, `tar.gz`, `7z` |
| `--password <PASS>` | Password for encryption (ZIP, 7Z) |
| `--level <N>` | Compression level 0-9 (default: 6) |

#### `ufop archive extract`

Extract an archive.

```bash
ufop archive extract backup.zip
ufop archive extract backup.zip --dest /tmp/extracted
ufop archive extract secure.zip --password mypassword
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `<ARCHIVE>` | Archive file to extract |

**Options:**

| Flag | Description |
|------|-------------|
| `--dest <DIR>` | Destination directory (default: current directory) |
| `--password <PASS>` | Password for encrypted archives |

#### `ufop archive list`

List archive contents.

```bash
ufop archive list backup.zip
ufop archive list backup.zip --json
```

**Arguments:**

| Argument | Description |
|----------|-------------|
| `<ARCHIVE>` | Archive file to list |

**Options:**

| Flag | Description |
|------|-------------|
| `--password <PASS>` | Password for encrypted archives |

---

### `ufop backup` repository format

Format commands of the versioned backup engine (all take `--dest`, `--prefix` and the passphrase options of `ufop backup run`). See [backup-repository-format.md](backup-repository-format.md).

```bash
ufop backup format-info --dest /mnt/backup                  # version, chunker, codecs, cipher, layout
ufop backup upgrade --dest /mnt/backup                      # format 1 → 2 (header only)
ufop backup upgrade --dest /mnt/backup --rollback           # back to format 1 before any format 2 write
UFOP_NEW_PASSPHRASE=… ufop backup change-password --dest /mnt/backup [--new-passphrase-file F] [--kdf-preset sensitive]
ufop backup reshard --dest /mnt/backup --layout sharded --shard-prefix-len 2 [--dry-run]
```

New backups (`ufop backup run`) take `--format-version 1|2`, `--chunker fixed|fastcdc`, `--block-codec deflate|zstd`, `--cipher aes256-gcm|chacha20-poly1305` and `--layout flat|sharded` with `--shard-prefix-len 1-4`. They apply only when the repository is created.

### `ufop backup` repository safety

Locks, read-only connections, checkpoints and the local database key. See [backup-safety.md](backup-safety.md).

```bash
ufop backup lock status --dest /mnt/backup                  # who holds the lock (exit 1 while in use)
ufop backup lock break --dest /mnt/backup --yes [--owner-id ID]   # only when the holder is no longer running
ufop backup restore --dest /mnt/backup --read-only --target ~/out # nothing is written, not even a lock
ufop backup run … --db-key-source keyring                   # encrypted local database, key in the OS keychain
ufop backup run … --db-key-source env:UFOP_DB_KEY           # key from an environment variable (64 hex digits)
ufop backup db encrypt ~/.local/share/ufop/cli-backups/x.sqlite [--db-key-source env:VAR] [--decrypt]
```

Every command with `--dest` also takes `--read-only`, `--db-key-source keyring|env:VAR|none` (default `none`; an encrypted database without its key is rebuilt from the destination), `--lock-ttl-secs` (default 1800), `--clock-skew-warn-secs` (300) and `--clock-skew-abort-secs` (3600; 0 turns a limit off). `ufop backup run` takes `--checkpoint-interval-secs` (default 900) and `--checkpoint-interval-bytes` (default 0); 0 turns them off.

### `ufop backup` versions, retention, copies and sharing

Notes, tags, pins, retention previews, count-based retention, copying versions and shared repositories. See [backup-versions.md](backup-versions.md).

```bash
ufop backup run … --note "before upgrade" --tag env=prod --tag release
ufop backup run … --keep-latest 2 --keep-daily 7 --keep-weekly 4 --keep-monthly 12 --keep-yearly 3
ufop backup list --dest /mnt/backup [--tag env=prod]          # table: time, files, size, added, pin, host, note, tags
ufop backup pin --dest /mnt/backup 3 [--reason audit]        # Keep forever
ufop backup unpin --dest /mnt/backup 3
ufop backup annotate --dest /mnt/backup 0 --note "release 2.1" [--tag k=v …] [--clear-tags]
ufop backup delete --dest /mnt/backup 3 --force              # pinned versions need --force
ufop backup retention preview --dest /mnt/backup --keep-daily 7   # kept/deleted per version, with the reason
ufop backup retention apply --dest /mnt/backup --keep-versions 10 [--dry-run] [--allow-full-removal]
UFOP_TO_PASSPHRASE=… ufop backup copy-versions --dest /mnt/backup 0 2 --to /mnt/offsite \
    [--to-prefix ufop] [--to-dbpath F] [--to-passphrase-env VAR] [--to-no-encryption] \
    [--to-format-version 1|2] [--to-chunker fixed|fastcdc] [--dry-run]
ufop backup run … --shared --job-id laptop-docs              # create a shared repository
ufop backup share --dest /mnt/backup [--keep-identities]     # share an existing format 2 repository
ufop backup sources --dest /mnt/backup                       # computers and jobs writing to it (* = this one)
ufop backup restore --dest /mnt/backup --source-id <id> --target ~/out   # another source's version
```

Every repository command also takes `--job-id` (the source a CLI job writes as; default derived from `--dest` and `--prefix`, so give each CLI job that shares a repository its own), `--hostname` and `--username` (record another computer or user, to continue its history) and `--source-id ID|*` (work on another source of a shared repository). `ufop backup run` also takes `--shared-gc-delay-hours` (default 4): in a shared repository, compaction leaves younger volumes alone. Any non-zero `--keep-latest|hourly|daily|weekly|monthly|yearly` selects count-based retention over `--keep-versions`, `--keep-days` and `--retention-policy`.

### `ufop backup` sources and runs

Ignore files, one filesystem, the read-error policy, rehash, standard input, the upload limit, parallel hashing, snapshot methods, the estimate and backing up every job. See [backup-sources-and-runs.md](backup-sources-and-runs.md).

```bash
ufop backup run … --ignore-file .ufopignore --ignore-file .backupignore --respect-gitignore
ufop backup run … --one-file-system                           # mount points inside a source stay empty
ufop backup run … --on-file-error fail --on-dir-error continue --on-unknown-entry warn [--fail-fast] [--max-errors 50]
ufop backup run … --rehash-percent 5                          # re-check ~5% of unchanged files per run
ufop backup run … --max-upload 20G                            # partial version at 20 GiB; the next run continues
ufop backup run … --parallel-file-threshold 512M              # 0 = off; blocks are identical either way
ufop backup run … --snapshot-policy required --snapshot-provider zfs   # auto | lvm | zfs | btrfs
pg_dump mydb | ufop backup stdin --dest /mnt/backup --name mydb.sql    # standard input as /stdin/mydb.sql
ufop backup estimate --dest /mnt/backup --source ~/Documents [--assumed-speed 20M] [--largest 10]
ufop server run-all [--job Docs …] [--scheduled-only] [--dry-run] [--wait]   # every job of the signed-in ufop-server
ufop backup run-all [--job Docs …] [--scheduled-only] [--wait]               # the same
```

| Flag | Default | |
|---|---|---|
| `--ignore-file NAME` | `.ufopignore` | Ignore-file name read in every folder (gitignore syntax; repeatable; replaces the default; `""` reads none) |
| `--respect-gitignore` | off | Also `.gitignore` and `.git/info/exclude` |
| `--one-file-system` | off | Do not enter folders on another filesystem |
| `--on-file-error` / `--on-dir-error` | `continue` | `continue` (warning) or `fail` (error) |
| `--on-unknown-entry` | `ignore` | Sockets, FIFOs, devices: `ignore`, `warn`, `fail` |
| `--fail-fast` | off | Stop at the first error with a partial version |
| `--max-errors N` | none | Stop after more than N read problems |
| `--rehash-percent N` | 0 | 0–100 |
| `--max-upload SIZE` | none | `500M`, `20G`, … (binary units) |
| `--parallel-file-threshold SIZE` | `256M` | 0 = off |
| `--snapshot-provider` | `auto` | Linux: `auto`, `lvm`, `zfs`, `btrfs` |

`ufop backup stdin` takes the repository, retention, `--note`/`--tag` and engine flags of `ufop backup run`. Exit codes follow `run`: 0 success, 1 warnings, 2 errors or a partial version (fail-fast, error budget, upload limit).

### `ufop backup` restore and reading

Restore as of a time, to an archive, mirror restores, owners, the restore error policy, sparse files, reading one file, the volume cache and serving a version as a drive. See [backup-restore.md](backup-restore.md).

```bash
ufop backup restore --dest /mnt/backup --target ~/out --time "2026-10-01 09:00"   # local time; also 2026-10-01 or RFC 3339
ufop backup restore --dest /mnt/backup --target ~/out --version oldest            # latest | oldest | N | <timestamp>
ufop backup restore --dest /mnt/backup --archive ~/docs.zip [/home/me/Docs]       # .zip | .tar | .tar.gz | .tgz
ufop backup restore --dest /mnt/backup --target ~/Docs --mirror [--yes]           # also delete what the version lacks
ufop backup restore … --restore-owner always --on-missing-data continue --fail-fast --max-errors 20 --sparse off
ufop backup cat --dest /mnt/backup /home/me/Docs/a.txt [--time …] [--offset 0 --length 4096] > a.txt
ufop backup mount --dest /mnt/backup [--version latest | --time … | --all] [--port 0]   # read-only WebDAV until Ctrl-C
ufop backup cache stats|clear [--cache-dir DIR]
```

| Flag | Default | |
|---|---|---|
| `--version` | `0` | A number (0 = newest), `latest`, `oldest`, or a version's timestamp / file-list name |
| `--time` | none | The newest complete version made at or before this time (wins over `--version`) |
| `--allow-partial` | off | `latest`, `oldest` and `--time` may pick interrupted versions |
| `--archive PATH`, `--archive-format` | none | Write an archive instead of files; format `zip`, `zip-stored`, `tar` or `tgz` (default: from the name) |
| `--mirror`, `--yes` | off | Delete extra files in the restored folders of `--target`; lists them and asks unless `--yes` (refused without a terminal) |
| `--restore-owner` | `auto` | `auto` (when root/administrator), `always` (fail otherwise), `never` |
| `--on-file-error`, `--on-dir-error`, `--on-missing-data` | `fail` | `fail` (error) or `continue` (warning) |
| `--on-metadata-error` | `warn` | `ignore`, `warn`, `fail` |
| `--fail-fast`, `--max-errors N` | off, none | Stop at the first error / after more than N problems |
| `--sparse` | `auto` | `auto` leaves holes for zero runs; `off` writes every byte |
| `--use-cache`, `--cache-dir`, `--cache-size` | off, user cache folder, `2G` | Read volumes through the local volume cache (`cat` and `mount` always use it; `--cache-size 0` turns it off) |

`ufop backup mount` prints the drive's address (`http://127.0.0.1:PORT/<token>/`) and password. `--bind` can serve another address; anyone who can reach it and has the token can read the files.

### `ufop backup` verification, templates and schedules

Scoped and sampled verification works on a repository directly. Templates, effective settings and schedule previews use the signed-in `ufop-server` (`ufop server login`). See [backup-policies-and-schedules.md](backup-policies-and-schedules.md).

```bash
ufop backup verify --dest /mnt/backup --version latest --path '*/Documents/*' --percent 10 --files-percent 5 --parallel 8 --max-errors 20 --json
ufop backup verify --dest /mnt/backup --structure          # index and listing only, no downloads
ufop backup template list
ufop backup template create --name Laptops --settings '{"retention":{"type":"keep_days","days":30},"options":{"compression_level":3}}'
ufop backup template create --file laptops.json            # a template, or an export holding one
ufop backup template update Laptops --settings '{"schedule":{"enabled":true,"cron":["30 1 * * *"]}}'
ufop backup template apply Laptops --job Docs             # the job is based on it too (it wins over earlier ones)
ufop backup template export [--template Laptops] [--out templates.json]
ufop backup template import templates.json
ufop backup template delete Laptops                       # refused while a job uses it
ufop backup effective Docs                                # every setting and where it comes from
ufop backup schedule-preview --cron "30 2 * * Mon-Fri" --cron "0 9 * * Sat" --count 5
```

| Flag (`verify`) | Default | |
|---|---|---|
| `--version` (repeatable) | all | A number (0 = newest), `latest`, `oldest` or a version's timestamp |
| `--path` (repeatable) | all | A folder (everything below it) or a wildcard |
| `--structure` | off | Check the index and listing only; nothing is downloaded |
| `--samples N` / `--percent N` / `--full` | `--samples 3` | Download N volumes / N % of the volumes in scope / every volume, checking every block |
| `--files-percent N` | 0 | Rebuild N % of the scoped files and compare their hashes |
| `--parallel N` | 0 (= 4) | Downloads and file checks at once |
| `--max-errors N` | 0 (no limit) | Stop after N problems |
| `--use-cache` (+ `--cache-dir`, `--cache-size`) | off | Read volumes from the local volume cache |

`verify` exits with 0 when it finds no problems and 2 when it finds some. With `--json` the full report is printed, including `problems` with the affected versions and paths.

### `ufop backup` destinations: request limits and point in time

Every `ufop backup` command with `--dest` takes request limits (0 = unlimited) and the listing parallelism for sharded layouts. See [backup-destinations.md](backup-destinations.md).

```bash
ufop backup run --dest /mnt/nas/backup --source ~/Documents --put-rate 5 --max-concurrent-requests 4 --list-parallelism 8
ufop backup open-at Office --point-in-time 2026-10-01T08:00:00Z [--name "Office before the attack"]
```

| Flag | Default | |
|---|---|---|
| `--list-rate` / `--get-rate` / `--put-rate` / `--delete-rate` | 0 | Most listing / download / upload / delete requests per second |
| `--max-concurrent-requests` | 0 | Most requests in flight at once |
| `--list-parallelism` | 1 | Shard folders listed at once (1–64) |

`open-at` works with jobs of the signed-in `ufop-server` whose destination is a versioned S3 bucket. It creates a read-only job that reads the bucket as it was at the given RFC 3339 time and prints its name, id and version count. Restore from it on the server or in the app.

### `ufop server shares`

Share links on a `ufop-server` started with `--share-root` (sign in first with `ufop server login`).

```bash
ufop server shares create /srv/share/q3 --name "Q3 reports" --expires-in-days 14 --max-downloads 20
ufop server shares create /srv/share/inbox --kind upload --password "$PW" --max-upload-mb 500
ufop server shares list --csv
ufop server shares log <id> -n 100
ufop server shares revoke <id>
```

`create` prints the full link once (only its hash is kept). Kinds: `download`, `upload` (drop box), `both`.

Folder policies (inherited by subfolders):

```bash
ufop server folders set /srv/share/inbox --allow pdf,docx --intake sequence --max-upload-mb 200
ufop server folders set /srv/share/archive --lock --require-password true --max-link-days 14
ufop server folders show /srv/share/inbox/2026     # own policy, the one in force and its sources
ufop server folders copy /srv/share/inbox /srv/share/hr
ufop server folders clear /srv/share/archive
ufop server folders list
```

---

### `ufop gateway`

An on-premises **file gateway**: serve folders on this machine through a `ufop-server`, so the server's users can browse, download and (optionally) upload them. The agent connects out to the server over HTTPS, so no inbound port is opened. Data streams straight through and the server does not store it. Start agents with the same `--name` on several machines to share the work and fail over.

```bash
# On the machine with the files (token: ufop server tokens create gw --scope write)
UFOP_GATEWAY_TOKEN=ufop_pat_… ufop gateway serve --server https://files.example.com \
    --name office --root /srv/projects --root /srv/scans [--writable] [--max-upload-mb 500]

# Anywhere, after `ufop server login`
ufop gateway list                                  # gateways and their nodes
ufop gateway ls office /projects                   # list a folder (/ = the roots)
ufop gateway get office /projects/q3/report.pdf    # download (-o to choose the file)
ufop gateway put office ./notes.txt /projects/notes.txt   # upload (writable gateways)
```

Paths are `/<root folder name>/…`. They stay inside the roots after symlinks are resolved, and dot-files are neither listed nor served. Uploads are written under a hidden temporary name and renamed into place. Ctrl+C stops an agent, which leaves the pool at once.

### `ufop mcp`

Serve file tools to an AI client (Claude Desktop, Claude Code, any MCP client) over the Model Context Protocol on stdio. Every path is confined to the `--root` directories: paths are resolved and symlinks followed before the check, so `..`, absolute paths and links pointing outside are refused. Read-only unless `--allow-write`.

```bash
ufop mcp --root ~/Projects/reports                       # read-only
ufop mcp --root ~/Inbox --root /srv/share --allow-write  # several roots, with write tools
```

Client configuration (e.g. `claude_desktop_config.json`):

```json
{ "mcpServers": { "ufop": { "command": "ufop", "args": ["mcp", "--root", "/home/me/Projects"], "env": { "UFOP_LOG_FILE": "/home/me/.ufop-mcp.log" } } } }
```

| Flag | Description |
|------|-------------|
| `--root <DIR>` | Directory the client may use (repeatable, required). Named after its last component (`docs`, `docs-2`, …). |
| `--allow-write` | Also offer `write_file`, `create_directory`, `copy_file`, `move`, `delete` |
| `--max-read-bytes <N>` | Largest read per call (default 4 MiB) |

For remote clients, `ufop-server --mcp-root <DIR>` serves the same tools over HTTP at `/mcp` with OAuth sign-in (see *MCP over HTTP* in the API reference).

Tools: `list_roots`, `list_directory` (optionally recursive; symlinks listed, not followed), `get_file_info`, `read_text_file` (byte ranges; `encoding: base64` for binary), `search_files` (glob on names), `checksum` (sha256/sha1/md5); with `--allow-write`: `write_file` (atomic; `overwrite` to replace), `create_directory`, `copy_file`, `move`, `delete` (`recursive` for non-empty directories; roots and symlinks cannot be deleted or written through). Paths are `<root>:relative/path`, absolute inside a root, or relative to the first root. Tool annotations mark read-only and destructive tools. Each call is logged (tool, arguments, outcome) to the log (`--log-file` / `UFOP_LOG_FILE`); stdout carries only protocol messages.

---

### `ufop status`

Show server and platform status.

```bash
ufop status
ufop status --json
```

Displays:

- Server connectivity status
- Platform information
- Client version
- Active transfers count
- Active sync pairs count

---

## Environment Variables

| Variable | Description |
|----------|-------------|
| `UFOP_API_URL` | Default API server URL |
| `RUST_LOG` | Logging level (overrides `-v` flags). Example: `RUST_LOG=debug` |

---

## Scripting Examples

### Backup Script

```bash
#!/bin/bash
# Daily backup script
DATE=$(date +%Y-%m-%d)
ufop archive create "/backups/backup-$DATE.tar.gz" /data --format tar.gz
ufop transfer "/backups/backup-$DATE.tar.gz" s3://my-bucket/backups/ --verify
echo "Backup completed: $DATE"
```

### Sync All Pairs

```bash
#!/bin/bash
# Run all sync pairs
for pair in $(ufop sync list --json | jq -r '.[].name'); do
  echo "Syncing: $pair"
  ufop sync run "$pair"
done
```

### Find and Report Duplicates

```bash
#!/bin/bash
# Report duplicates as JSON
ufop duplicates /shared/drive --min-size 1048576 --json > duplicates-report.json
echo "Found $(cat duplicates-report.json | jq length) duplicate groups"
```
