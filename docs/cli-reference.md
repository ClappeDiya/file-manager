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
| `--format <FORMAT>` | Output format: `human` (default), `json`, `yaml` |
| `--json` | Shorthand for `--format json` |
| `--yaml` | Shorthand for `--format yaml` |
| `--dry-run` | Show what would happen without executing |
| `--limit <RATE>` | Bandwidth limit (e.g., `1M`, `500K`, `2G`) |
| `--api-url <URL>` | API server URL (or set `UFOP_API_URL` env var) |
| `-v`, `-vv`, `-vvv` | Verbosity level (info, debug, trace) |

### Exit Codes

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Partial failure |
| 2 | Full failure |
| 3 | Authentication error |
| 4 | Policy violation |

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
