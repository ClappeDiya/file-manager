# Unified File Operations Platform - User Guide

Version 0.1.0

---

## Table of Contents

1. [Getting Started](#getting-started)
2. [Simple Mode](#simple-mode)
3. [Advanced Mode](#advanced-mode)
4. [File Browsing](#file-browsing)
5. [File Operations](#file-operations)
6. [Transfers](#transfers)
7. [Sync](#sync)
8. [Cloud & Server Connections](#cloud--server-connections)
9. [Peer-to-Peer Transfers](#peer-to-peer-transfers)
10. [Archive Operations](#archive-operations)
11. [Batch Rename](#batch-rename)
12. [File Preview](#file-preview)
13. [Integrity Tools](#integrity-tools)
14. [Terminal](#terminal)
15. [AI Assistant](#ai-assistant)
16. [Encrypted Vaults](#encrypted-vaults)
17. [Cross-Platform Compatibility](#cross-platform-compatibility)
18. [Keyboard Shortcuts](#keyboard-shortcuts)
19. [Themes & Accessibility](#themes--accessibility)

---

## Getting Started

### Installation

UFOP is available as a native desktop application for macOS, Windows, and Linux.

- **macOS**: Download the `.dmg` installer (requires macOS 10.15 or later)
- **Windows**: Download the NSIS or WiX installer (`.exe` or `.msi`). The installer supports multiple languages (English, Spanish, French, German, Portuguese, Japanese, Chinese, Korean).
- **Linux**: Available as `.deb`, `.rpm`, or `.AppImage`

### Portable Mode (run from a USB drive)

Portable mode keeps **everything** the app stores in one folder you can carry around: the settings database, backup job databases, the search index, logs, the backup volume cache, thumbnails and previews, sync state, editor drafts, temporary files and saved credentials. Nothing is written to the computer's user profile.

Turn it on in one of three ways (the first that applies wins):

1. Set the environment variable `UFOP_PORTABLE_DIR` to a folder (a relative path is relative to the app's folder).
2. Put `ufop-portable.json` next to the app: `{ "data_dir": "UFOP Data" }` (relative to the app's folder; default `ufop-data`).
3. Put an empty file named `portable` next to the app. The data goes into `ufop-data` next to it.

"Next to the app" is the folder that holds the executable (`FileManager.exe` on Windows, the AppImage or binary on Linux); on macOS it can also be the folder that holds the `.app` bundle. Restart the app after adding the file. **Settings** then shows *Portable mode: on (path)* in the header, and **Settings → Storage & Portable Mode** lists the folders and where credentials are kept.

Inside the portable folder: `data/` (databases, backup job databases, `data/logs`), `config/` (credential vault), `cache/` (volume cache, thumbnails, mounts) and `tmp/`. On Windows the WebView2 profile goes to `webview/`, on Linux the WebKit data to `xdg/`; on macOS the system web view cannot be pointed at a folder, so portable mode gives the window its own web data store, identified by the portable folder's path: each portable copy keeps its browser storage (window and panel preferences) apart from the installed app and from other copies. That store stays in the user profile (`~/Library/WebKit`), and needs macOS 14 or later; on older macOS the window uses the default store, shared with the installed app.

**Credentials.** The system keychain cannot travel with the drive, so in portable mode passwords and keys are saved in an encrypted vault file, `config/credentials.vault` (AES-256-GCM). By default its key is a random key file next to it (`config/credentials.key`): that keeps secrets out of plain text, but anyone with the drive can open the vault, so use an encrypted drive. For real protection set `UFOP_PORTABLE_PASSPHRASE` before the first start; the vault key is then derived from the passphrase (Argon2id) and the app needs the same passphrase on every start. Credentials saved in the keychain before switching to portable mode are not copied; enter them again.

`ufop-server` honours `UFOP_PORTABLE_DIR` and the marker files the same way when `--data-dir` is not given.

### First Launch - Onboarding Wizard

On first launch, a 6-step onboarding wizard guides you through setup:

1. **Welcome** - Introduction to the platform
2. **Choose Your Style** - Select how you'll use the app:
   - **Personal**: Simple, clean interface for managing photos, documents, and music
   - **Power User**: Advanced mode with regex renaming, checksums, scripting, and logs
   - **Workplace**: Team features with server connections and compliance controls
3. **Connect Locations** - Link cloud services (OneDrive, Google Drive, Dropbox) or local folders
4. **Quick Start** - Choose your first action (transfer, sync, backup, or browse)
5. **Compatibility** - Learn how UFOP handles cross-platform file naming
6. **Ready** - Enter your workspace

You can skip the wizard at any step, and re-run it later from **Help > Getting Started** or **Settings**.

---

## Simple Mode

Simple mode is the default experience. It provides a streamlined interface with larger touch targets, more whitespace, and only the features most people need.

### Simple Mode Sidebar Navigation

- **Files** - Browse and manage your files
- **Transfers** - Send and receive files between locations
- **Sync** - Keep two folders automatically synchronized
- **Cloud & Servers** - Connect to OneDrive, Google Drive, Dropbox, or remote servers
- **Search** - Find files and folders across all connected locations
- **Favorites** - Quick access to saved locations
- **Activity** - View recent operations and their results
- **Settings** - Customize your experience (theme, mode, getting started)

### Switching to Advanced Mode

At the bottom of the Simple sidebar, click **More Options** to switch to Advanced mode. You can switch back at any time -- no restart needed. This can also be changed in **Settings > User Mode**.

---

## Advanced Mode

Advanced mode unlocks the full feature set:

- Dual-pane file browsing
- Tabbed navigation per pane
- Regex-based batch renaming
- Checksum verification
- Terminal panel
- Detailed transfer logs
- Command palette (Ctrl/Cmd+K)
- Filter bar and sorting controls
- Data grid view with virtual scrolling

---

## File Browsing

### Views

Files can be displayed in four modes (set per pane in Advanced mode):

- **List** - Compact rows with file names
- **Detail** - Full rows with name, size, date, and permissions
- **Grid** - Thumbnail grid for visual browsing
- **Compact** - Dense listing for maximum visibility

### Navigation

- **Breadcrumb Bar** - Click any segment to navigate up the directory tree
- **Tab Bar** - Open multiple directories in tabs (Advanced mode). Tabs can be pinned, reordered, and closed.
- **Favorites** - Star any folder for quick access from the sidebar
- **Recent Locations** - The last 20 visited paths are remembered

### Selection

- Click to select a single file
- Ctrl/Cmd+Click to toggle individual items
- Shift+Click to select a range
- Ctrl/Cmd+A to select all

### Dual Pane (Advanced Mode)

In Advanced mode, the workspace splits into two panes. Each pane has its own tabs, view mode, sorting, and filter settings. The split ratio is adjustable between 20%-80%.

---

## File Operations

All file operations support undo (up to 10 levels).

| Operation | Description |
|-----------|-------------|
| **Copy** | Copy files to a destination. Preserves originals. |
| **Move** | Move files to a destination. Removes originals. |
| **Rename** | Rename a single file or folder. |
| **Duplicate** | Create a copy in the same directory with a suffix. |
| **Delete** | Move to trash (or permanent delete if configured). |
| **Create Folder** | Create a new directory. |
| **Create File** | Create a new empty file. |
| **Undo** | Reverse the last file operation. |

### Context Menu

Right-click (or long-press on touch) to access the context menu with all file operations, plus:

- Open in Terminal
- Copy path
- Get Info / metadata
- Tags and labels
- Smart Folder actions

---

## Transfers

### Enqueuing a Transfer

Transfers can be started via:

- Drag-and-drop between panes or from external apps
- The "Start a Transfer" guided wizard (Simple mode)
- Right-click > Transfer to...
- The command palette
- The CLI (`ufop transfer`)

### Transfer Queue

Active and queued transfers appear in the Transfer panel. Each transfer shows:

- Source and destination
- Progress bar with percentage and speed
- Estimated time remaining
- Priority (High / Normal / Low)

### Transfer Controls

- **Pause / Resume** - Temporarily stop and restart a transfer
- **Cancel** - Abort a transfer
- **Retry** - Re-attempt failed transfers (with configurable retry policy)
- **Priority** - Reorder the queue by setting High/Normal/Low priority
- **Reorder** - Drag transfers in the queue to change execution order

### Bandwidth Throttling

Set global or per-connection bandwidth limits:

- Global throttle applies to all active transfers combined
- Per-connection throttle limits individual connections

### Conflict Resolution

When a destination file already exists, choose from:

- **Ask** - Prompt for each conflict
- **Overwrite** - Always replace existing files
- **Skip** - Skip conflicting files
- **Rename** - Auto-rename with a suffix
- **Newer Wins** - Keep the file with the more recent modification date

### Verification

Post-transfer checksum verification ensures data integrity. Supported algorithms:

- MD5
- SHA-1
- SHA-256

A three-tier verification system is available:
1. **Quick** - File size comparison only
2. **Standard** - xxHash3 fast checksum
3. **Full** - SHA-256 cryptographic hash

### Transfer History

All completed transfers are logged and searchable. History can be exported as CSV.

---

## Sync

### Creating a Sync Pair

A sync pair links two folders and keeps them synchronized. Configure:

- **Name** - A friendly label for the sync pair
- **Source & Destination** - Local paths, remote connections, or cloud storage
- **Mode** - Bidirectional, source-to-destination, or destination-to-source
- **Trigger** - Manual, on-change (filesystem watcher), or scheduled (cron expression)
- **Conflict Policy** - How to handle files changed on both sides
- **Filters** - Include/exclude patterns, file types, size limits
- **Verification** - Enable checksum comparison for changed files

### Running a Sync

- **Manual Run** - Click "Run Sync" to start immediately
- **Dry Run** - Preview what will happen without making changes. Results can be exported as CSV.
- **Watcher** - Start a filesystem watcher that triggers sync on changes
- **Scheduled** - Define a cron expression for automatic runs

### Sync Health

Each sync pair has a health indicator:

- **Green** - Last run succeeded with no issues
- **Yellow** - Completed with warnings or minor conflicts
- **Red** - Last run had errors
- **Gray** - Never run or status unknown

### Conflict Resolution

When files conflict during sync, they are quarantined. Resolve via:

- **Keep Source** - Use the source version
- **Keep Destination** - Use the destination version
- **Keep Both** - Rename one copy
- **Skip** - Leave unchanged

### Rollback

If a sync produced unwanted results, use the Rollback feature to restore files from the pre-sync backup.

### Reports

Every sync run generates a report with:

- Files added, modified, deleted, skipped
- Conflicts resolved and errors
- Bytes transferred and duration
- Health status

Reports can be exported as CSV or JSON.

---

## Cloud & Server Connections

### Supported Protocols

| Protocol | Description |
|----------|-------------|
| **SFTP** | SSH-based secure file transfer. Supports key auth, jump hosts, and host key verification. |
| **FTP/FTPS** | Classic FTP with active/passive mode and optional TLS encryption. |
| **WebDAV** | HTTP-based DAV protocol. Compatible with Nextcloud, ownCloud, and other DAV servers. |
| **SMB/CIFS** | Windows file shares and NAS devices. Supports LAN share discovery. |
| **NFS** | Network File System (v3/v4). Displays Unix permissions. |
| **Amazon S3** | S3 and compatible services (MinIO, Wasabi, B2 S3 mode). |
| **Google Drive** | OAuth 2.0 login. Supports My Drive, Shared Drives, Docs export, and versioning. |
| **Dropbox** | OAuth 2.0 login. Team/shared folders and session uploads. |
| **OneDrive** | Personal and Business accounts. SharePoint support. Handles naming restrictions. |
| **Backblaze B2** | Native API with large file uploads. S3 compatibility mode available. |
| **Azure Blob Storage** | Containers and blobs; uploads in 4 MiB blocks; folders as `dir/` marker blobs; read-only SAS share links. |
| **Azure Data Lake Storage Gen2** | Accounts with a hierarchical namespace (`dfs` endpoint): real folders, and uploads that appear only when complete (written under a hidden name, then renamed). |
| **Azure Files** | File shares over the REST API (no SMB port needed); uploads in 4 MiB ranges under a hidden name, then renamed; read-only SAS share links. |
| **OpenStack Swift** | Containers and objects; Keystone v3 (password or application credential) or TempAuth; files over 32 MiB upload as Static Large Objects; folders as `dir/` objects; TempURL share links. |

For the Azure services, enter the **storage account** and sign in with an **account key**, a **SAS token** or a **connection string**. Leave **Endpoint** blank for the public cloud; enter a host or URL for Azure Government, Azure China or the Azurite emulator. A container, file system or share is never deleted from the app (that removes everything in it); folders are removed only when empty.

For **OpenStack Swift**, choose how to sign in:

- **Keystone user name and password**: enter the user, password, **project name** and **domain** (default `Default`). The domain applies to both the user and the project.
- **Keystone application credential**: enter the **application credential ID**, and its secret as the password. The credential's own project applies.
- **TempAuth (v1)**: enter the user as `account:user` and the key as the password.

**Auth URL** is the Keystone (or TempAuth) URL; leave it blank to use `https://<host>:<port>/v3` (or `/auth/v1.0` for TempAuth). A URL without a version gets `/v3`. When the project has storage in several regions, enter the **region** to use. Large objects keep their segments in `<container>_segments`; replacing or deleting the object removes its old segments. Containers and folders are removed only when empty. Share links are TempURLs and need a `Temp-URL-Key` on the account or the container (for example `swift post -m Temp-URL-Key:<secret>`); changing that key revokes every link.

### Connection Management

- **Save** - Store connection profiles with names and groups
- **Test** - Verify connectivity before saving
- **Groups** - Organize connections into named groups
- **Import/Export** - Share connection profiles between machines
- **Credential Storage** - Passwords and keys stored in the OS keychain (macOS Keychain, Windows Credential Manager, Linux Secret Service)

### Guided Wizards

Simple mode provides step-by-step wizards for:

- Connect OneDrive / Google Drive / Dropbox
- Set up local sync
- Connect to Windows Share (SMB)
- Connect via SFTP

---

### Mounting Drives

*Mount drive* turns a saved connection into a folder (SMB, NFS and SFTP through the OS; cloud storage through rclone + FUSE). Cloud drives have cache options:

- **Cache mode** — *Off* (always from the server), *Minimal* (open files only), *Writes* (default: uploads finish in the background), *Full* (whole files are kept, so they stay readable offline).
- **Largest cache** and **Keep cached files** limit the space and age of the local copies; **Read-only** refuses changes.
- With the full cache, **Make available offline** downloads a folder (or the whole drive) into the cache; the cache size shows per drive, and **Clear cache** frees it once the drive is unmounted.
- Your organization's **mount profile** (from the management console) may allow only some kinds of drives, require read-only mounts or a cache mode, cap the cache, or require mount points inside a folder; the dialog shows it and locks the options it forces.

## Peer-to-Peer Transfers

Send files straight to another computer running UFOP on the same network, over an encrypted connection. Open **Nearby Computers** from the toolbar (add it in the toolbar customizer) or the command palette.

### Discovery

Click **Start Scanning**: UFOP advertises this computer over mDNS (Bonjour/Avahi) and lists the other UFOP computers it finds, with their name, operating system and status. While scanning, this computer also accepts incoming requests (it listens on port 43210, or another free port if that one is taken). Stop scanning to stop both. A computer that stops scanning disappears as offline.

### Identity and security

Each computer has its own certificate, created on first use; its fingerprint is shown at the bottom of the panel ("This device: AB12 CD34 …"). Connections use TLS with both computers presenting their certificates, so each side knows who it talks to. When you trust, block or save a computer, its certificate is remembered: if a saved computer later shows a different certificate (for example after a reinstall, or an impostor), the connection is refused until you remove it from saved devices. Compare fingerprints on both screens the first time if the network is not your own.

### Trust Levels

- **Unknown / Untrusted** - you approve each transfer
- **Trusted** - transfers are accepted automatically into your Downloads folder
- **Blocked** - connections and requests are refused

### Transferring to Peers

1. Select files or folders and choose **Send to Another Computer…** (right-click menu or command palette).
2. Pick the computer in the panel and click its send button.
3. The recipient sees the request with the sender, file count and size, and chooses **Accept** (into Downloads), **Save here** (the folder open in their file manager), **Deny**, or block the sender. Requests not answered within 5 minutes are declined.
4. Files stream in 4 MiB pieces; each file is checked with SHA-256 and kept under a hidden name until it arrives intact, so a cancelled or failed transfer leaves no partial files. Existing files are never overwritten (copies are named `name (1).ext`), folders keep their structure, and modification times are kept. Symbolic links are skipped.

Either side can cancel; progress, speed and the result show under **Transfers** on both computers.

### Manual Connection

If discovery doesn't find a computer (different subnet, multicast blocked), click **+** and enter its IP address or host name and port. The other computer must be accepting transfers (scanning).

---

## Archive Operations

### Supported Formats

- ZIP (with optional AES encryption)
- TAR
- TAR.GZ (gzip compressed tar)
- 7Z (with optional encryption)

### Operations

- **Create** - Package files into an archive with configurable compression level (0-9)
- **Extract** - Unpack archives to a destination directory
- **Browse** - View archive contents without extracting
- **Info** - Display archive metadata (size, file count, compression ratio)

---

## Batch Rename

Rename multiple files using patterns or find/replace:

### Pattern Tokens

| Token | Description |
|-------|-------------|
| `{name}` | Original filename without extension |
| `{ext}` | File extension |
| `{num}` | Sequence number |
| `{date}` | File modification date |
| `{parent}` | Parent directory name |
| `{counter}` | Auto-incrementing counter |

### Options

- **Find/Replace** - Literal or regex-based string replacement
- **Case Transform** - Convert to upper, lower, title, or sentence case
- **Counter** - Configurable start value, step, and zero-padding
- **Preview** - See all proposed changes before applying
- **Undo** - Revert the entire batch rename operation

---

## File Preview

The preview pane (toggled from the toolbar or View menu) renders:

- **Images** - Inline preview with EXIF metadata display
- **Text Files** - Syntax-highlighted content preview
- **Audio/Video** - Playback controls
- **PDF** - Rendered in place
- **Office documents** (Word, Excel, PowerPoint, OpenDocument, RTF; legacy `.doc`/`.xls`/`.ppt`) - The document's text appears at once (spreadsheets as rows of cells, presentations slide by slide); the page itself is rendered by Quick Look on macOS or by LibreOffice when it is installed (any platform; set `UFOP_SOFFICE` to point at a non-standard install), then shown as a PDF with *Page* / *Text* tabs. Renders are cached until the file changes. Without a renderer the text stays, with *Open in default app*.

---

## Integrity Tools

### Checksums

Compute and verify checksums for selected files using MD5, SHA-1, or SHA-256.

### Duplicate Finder

Scan a directory to find duplicate files by content hash. Actions:

- **Report** - List all duplicates
- **Keep Newest** - Delete all but the most recently modified copy
- **Keep Largest** - Delete all but the largest copy
- **Delete** - Remove specific duplicates

### Tags and Labels

Organize files with custom tags and color labels:

- Create, rename, and delete tags
- Tag/untag individual files
- Set color labels on files
- View file info including all tags and labels

### Metadata and Metadata Schemas

Files carry key-value metadata (client, invoice number, status …), edited under Tags & Labels and searchable with *Find by metadata*. It follows files that are moved or renamed in the app.

**Metadata Schemas** (Integrity Tools → Metadata Schemas) define the fields a folder's files carry:

- Field types: text (optional pattern and length), number, whole number (with ranges), date, yes/no, choice (fixed options), email and URL; each field can be required.
- A schema applies to its folder and, with *Include subfolders*, everything below it; a subfolder can override it with its own schema (the nearest one wins).
- Values are checked and normalized when they are set (e.g. `yes` → `true`, choices take the option's spelling); required fields cannot be removed; a *strict* schema accepts only its own fields.
- The metadata editor shows the schema's fields first with matching inputs and highlights missing required ones.
- *Check files* lists files that miss required fields or hold values that no longer fit (set before the schema existed or changed).

### Smart Folders

Create virtual folders based on rules (file type, size, date, tags). Smart folders update dynamically as files change.

---

## Terminal

### Local Terminal

Open a terminal session in the current directory. Supports:

- Default shell detection (bash, zsh, fish, etc.)
- Multiple sessions with tab switching
- Split layouts (horizontal or vertical)
- Path escaping for drag-and-drop into the terminal

### Remote Terminal

Connect to remote servers via SSH directly from UFOP. Uses saved connection profiles for authentication.

---

## AI Assistant

### Chat and Help

Ask questions about files, get help with operations, or troubleshoot errors. Answers come from this documentation, which is built into the app: with a local model running (Ollama, `UFOP_OLLAMA_URL` for another address) the assistant writes an answer from the best sections; without one it shows those sections. The **Help** tab lists the sources of every answer and works offline, even with AI turned off.

### Activity report

The **Report** tab summarizes the last 24 hours, 7 or 30 days: rule runs and failures per rule (failing rules first, with their latest error), file operations by kind and size, and what needs attention. With a local model, a short summary is added — written from the report's numbers only. Copy it or save it as a Markdown file.

### Unattended AI tasks

The automation action **Ask the local AI** (Advanced) applies an instruction — "summarize", "classify as invoice/receipt/other", "extract the total" — to each file's text (PDFs and office documents included) and saves the answer as `<file name>.ai.md` (or another suffix) next to the file or in a chosen folder. It only runs when AI and *content analysis* are allowed in the AI settings, uses the local model, and every task is recorded in the AI audit log.

### Error Explanation

When an error occurs, the AI explains:

- What happened (in plain language)
- Possible causes
- Suggested fixes
- Related documentation

### Suggestions

The AI proactively suggests actions based on your context (e.g., "You have duplicate files in this folder" or "This sync pair hasn't run in 7 days").

### Natural Language Job Creation

Describe what you want in plain English and the AI creates the transfer, sync, or rename job for you. Example: "Sync my Documents folder to Google Drive every night at 10pm."

### Safety Controls

- **Confirmation Required** - Destructive actions require explicit approval
- **Audit Log** - All AI actions are logged with input/output summaries
- **Feature Toggles** - Enable/disable AI features individually
- **Model Routing** - Choose between local and cloud AI models

---

## Encrypted Vaults

### Creating a Vault

Create an encrypted vault to store sensitive files. Vaults use AES-256-GCM or ChaCha20-Poly1305 encryption with Argon2 key derivation.

### Operations

- **Create Vault** - Specify a name and password
- **Unlock / Lock** - Open and close vaults with password
- **Encrypt / Decrypt** - Add and retrieve files from the vault
- **Change Password** - Update the vault password
- **Encrypt for Upload** - Encrypt files before sending to cloud storage
- **Decrypt from Download** - Decrypt files received from cloud storage

### Encryption Policies

Define organizational policies for encryption:

- Require encryption for specific file types
- Enforce encryption for uploads to specific destinations
- Check transport security (TLS/HTTPS enforcement)

---

## OpenPGP (GPG)

With GnuPG installed, files can be encrypted (to keys or with a passphrase), decrypted, signed and verified from the context menu; outputs are written next to the originals under free names.

- **Re-encrypt** (Advanced, on `.gpg`/`.pgp`/`.asc` files) changes who can open a file — other keys, or a new passphrase — in place. The file is decrypted in memory (up to 512 MB) and written again; no unencrypted copy is created, and a failure leaves the file unchanged.
- **Legacy files** made without integrity protection (common before 2003) are refused, because changes to them can't be detected. *Decrypt anyway* opens them when you trust the source, and the result says it was unprotected. Re-encrypting such a file produces a protected one.
- **Create a key pair** in Settings → OpenPGP Keys: Ed25519 (recommended) or RSA 4096, with an expiry and a passphrase; both can sign and encrypt.
- Passphrases go to gpg on its input, never on the command line, and plaintext is never written to a temporary file.

## Cross-Platform Compatibility

UFOP includes a compatibility engine that helps files work across macOS, Windows, Linux, and cloud services.

### Automatic Detection

The app warns when a filename might cause problems on another platform. Examples:

- Reserved names on Windows (CON, PRN, AUX, NUL, COM1, LPT1, etc.)
- Invalid characters (`:`, `<`, `>`, `|`, `?`, `*`, `"` on Windows)
- Path length limits (260 characters on older Windows)
- Trailing dots or spaces
- Unicode normalization differences (NFC vs NFD)

### Safe Renaming

During transfers, UFOP can automatically fix problematic names while preserving the originals. Every rename can be undone.

### Compatibility Badges

Files and folders display compatibility badges indicating their status across target platforms.

---

## Schedules & holiday calendars

In Advanced mode, **Schedules & holiday calendars** (toolbar or command palette) defines schedules and calendars once and shares them:

- A **named schedule** is a cron expression (`sec min hour day month weekday`, UTC) plus the holiday calendars whose days it skips. Choose it in a sync pair (Trigger: Scheduled) or an automation rule (Trigger: Schedule) instead of typing cron. Editing the schedule changes every pair and rule that uses it at once. A schedule that is still in use cannot be deleted.
- A **holiday calendar** lists days off: dates, dates that recur every year (`MM-DD`), and optionally weekends. Import an iCalendar (`.ics`) file to add its all-day events; yearly events recur. Backup jobs pick calendars on their Schedule step.
- A scheduled run that falls on a day off (by local date) is skipped, not postponed: the next regular run happens as usual. The schedule editor previews the next runs and marks the ones a calendar skips.

## Search: file contents

Folders added to the search index with *include file contents* are searchable by the text inside their files: plain-text files, PDFs, and Word, Excel, PowerPoint, OpenDocument and RTF documents (up to 1 MB of text per file). PDF text comes from the document's fonts, so scanned pages (images) and password-protected PDFs have no text; legacy `.doc`/`.xls`/`.ppt` files are found by name only. Folders indexed before document text was supported read their documents again on the next index run.

## Automation: workflows

In Advanced mode, a rule's actions can be edited as a **workflow**: a graph instead of a list (rule editor → *Edit as a workflow*; a plain chain turns back into a list with *Use a list of actions instead*).

- **Steps:** an action (any automation action), a **condition** (matching files continue on its *true* branch, the rest on *false*), a **delay**, a **join**, or a **nested rule** that runs another rule's actions on the files.
- **Connections** fire on success (default), on failure, always, or on a condition's true/false branch. Branches run in parallel. A step waits until all its inputs have finished, then runs when at least one of them fired; a join set to *every* needs all of them. Otherwise it is skipped, and so is everything after it.
- **Files flow along connections.** A step receives the files its inputs carry (several branches are merged), and an action runs once per file. Moving, renaming or downloading hands the new file on; other actions pass their input through.
- **Run graph:** a workflow run in the run history has a *Run graph* button that shows which steps succeeded, failed or were skipped.
- **History:** every edit keeps the previous version of the rule (up to 50). Open the rule's *History* to restore one. Command approvals must be given again after a restore.
- Workflows are checked when saved: no loops, known steps only, delays up to 24 hours, nested rules at most 4 deep and never running themselves. Programs (`Run a program`) inside a workflow need the same approval as in a list.
- **Organization workflows:** rules your organization pushes from the admin console show an *Org* badge. They run like your own but can only be viewed, not edited, switched off or deleted; *Clone* makes an editable copy. *Copy as JSON* (rule editor) copies any rule's definition — without its run statistics, command approval or webhook secret — to paste into the console.

## Converting images and documents

Right-click images (PNG, JPEG, WebP, GIF, BMP, TIFF) or documents (Word, Excel, PowerPoint, OpenDocument, RTF, text, HTML, CSV) → **Convert…**:

- **Images:** choose a format (JPEG with a quality, PNG, WebP, GIF, BMP, TIFF, or keep the format) and a largest width/height. Images shrink to fit, keeping their proportions, and are never enlarged. JPEG output drops transparency.
- **Documents:** convert to PDF, Word, OpenDocument, RTF, text, HTML, Excel, CSV or PowerPoint with LibreOffice, which must be installed (or set `UFOP_SOFFICE` to its `soffice` program). A private LibreOffice profile is used, so an open LibreOffice window isn't disturbed.
- Converted files are new files next to the originals (or in the folder you choose) under free names; the originals are never changed.

In automation rules, **Resize / convert an image** and **Convert a document** do the same for each file, into a folder template (empty = next to the file), following the rule's overwrite choice. Later steps work on the converted file, so *convert to PDF → upload* is two steps.

## Automation: validating a batch

In Advanced mode, **Validate a batch of files** checks a delivery — the files directly inside one folder (by default, the folder of the file that started the run; start the rule on a marker such as `*.done` so the batch is complete):

- **Member rules:** files matching a name pattern (`orders_*.csv`, case-insensitive) must number at least / at most N and each be at least a size. *Other files make the batch invalid* rejects anything no rule names.
- **Content checks** on matching files: not empty; valid JSON; CSV with required columns (case-insensitive), a minimum number of data rows and a delimiter; or a SHA-256 manifest (`sha256sum` format) whose listed files must exist in the batch and match.
- **Report:** with a report folder (templates allowed), each run writes `<folder>-<time>.validation.json` and `.txt` listing every check. An invalid batch fails the step — the run history shows which checks failed, and a workflow's failure branch can move the batch to quarantine or send a message. Turn that off to only report.

## Automation: scanning for malware

**Scan for malware (ClamAV)** checks each file with ClamAV — `clamdscan` when the clamd daemon runs (fast), else `clamscan`; set `UFOP_CLAMSCAN` for another location. A clean file goes on to the rule's next steps. An infected one is moved to the quarantine folder (owner-only, with a `.quarantine.json` note naming the signature and where it came from), you get a notification, and the run stops with an error naming the signature. If ClamAV is missing or the scan fails, the run fails too — nothing is passed on unscanned. The template **Scan arrivals for malware** sets this up for a folder; keep the quarantine folder outside the watched folder.

## Automation: downloading a URL

In Advanced mode, an automation rule can **Download a URL** into a folder. The URL, folder, file name and expected SHA-256 accept the same templates as other actions (`{name}`, `{yyyy}`, webhook fields …).

- The file name comes from the rule, else the server's `Content-Disposition`, else the URL. An existing file is kept and the new one gets a number, unless **Replace** is ticked.
- The download goes to a hidden temporary file and is moved into place when complete. It stops at the size limit (default 1024 MB), and a wrong checksum discards it.
- Up to 5 redirects are followed. Loopback, private, link-local (including cloud metadata) and other non-public addresses are blocked unless **Allow intranet addresses** is ticked. The check uses the address actually connected to, so it applies to every redirect too.
- Network failures are retried with the rule's retry settings; blocked addresses, missing pages and checksum mismatches are not.

## Automation: step results, risk and new steps

- **Step results (Advanced):** give a step a result name (*Save this step's result as*) and later steps can use it in any template field: `{steps.<name>.files}` (one path per line), `{steps.<name>.count}`, `{steps.<name>.bytes}` and `{steps.<name>.path}` (the first file). Filters work as usual (`{steps.listing.count|upper}`). A name must start with a letter and use letters, digits, `_` or `-`; each name is used once. A reference to a name no earlier step declares (in a workflow: no step upstream along the connections) is refused when the rule is saved or run, not halfway through a run.
- **Risk badges (Advanced):** each step shows a *Low*, *Medium* or *High risk* badge (hover for the reason) in the rule editor and the workflow builder. Permanent delete, *Run a program* and *Purge a folder* are High. Saving a rule that contains a High step (in Standard mode too, for example a permanent delete) first asks you, in the editor, whether to save it anyway; if the check itself cannot be done, you are asked about the whole rule; *Go back* saves nothing. At run time the safety interlock still checks destructive steps against what you normally do.
- **Verify a sync pair:** compares both sides of a sync pair and fails the step when files differ or are missing (*compare file contents* reads every file; otherwise only file sizes are compared). It changes nothing, so a workflow's failure branch can notify you.
- **Write a file listing:** writes a CSV or JSON list of the files under a folder (optionally with subfolders and folders) to the file you name; a listing inside the listed folder is not listed in itself. The result is available as `{steps.<name>.path}`.
- **Remove empty folders:** removes folders with nothing in them, deepest first (optionally the folder itself); a folder holding any file stays.
- **Purge a folder (Advanced):** removes a folder and everything in it, permanently. Drive roots and home folders are refused, an optional file limit stops a purge of a folder that grew unexpectedly, and the safety interlock stops a purge far larger than usual. Always try a dry run first.
- **Download straight to a remote:** *Download a URL* can send the file to a saved connection (*Send it to* → the connection, plus a remote folder template). The file is fetched to a temporary file, uploaded the same way as *Transfer to a connection*, and the temporary file is removed whether the upload succeeds or not.
- **Template search (Advanced):** *Start from a template* is a search box: type part of a name, description or topic (for example "antivirus" or "csv") and pick with the arrow keys and Enter. Standard mode keeps the plain list.

## Next run, and notifications for sync pairs and rules

- **Next run:** a scheduled sync pair (Sync panel) and a scheduled automation rule (Quickflows panel) show *Next run: in 12 min / Mon, 26 Oct, 02:00*. Holiday calendars are taken into account (a day off is skipped), disabled pairs and rules show nothing, and the tray menu has a line for what runs next.
- **Notify when a run finishes:** a sync pair (the *Notify* button on the pair, or the create form) and an automation rule (rule editor → Options) can send the result to a **notification profile** from Backup → Settings → Notification profiles — the same email, Slack, Teams, Telegram, Discord, XMPP and webhook channels backup jobs use. Tick which results send (success, warnings, errors); none ticked means whatever the profile itself sends. Choosing *Nobody* turns it off and clears the ticks and the size limit. Edits to a sync pair's *Notify* box are saved once you stop typing. A profile that a sync pair or rule uses cannot be deleted until you remove it there (or delete it with force, which clears those settings). In Advanced mode, *Don't report successful runs below N MB* mutes small successful runs — errors and warnings are always reported.
- **Desktop notifications for transfers and sync:** Settings → Notifications now takes effect: *Transfer complete / failed* and *Sync complete / failed* raise a system notification when a transfer or sync run ends (and *Only when backgrounded* keeps them quiet while the window is in front).

## Remote control from chat (Advanced, off by default)

Settings → Advanced → *Remote control from chat* lets you list, start and stop your backup, sync and automation jobs from Telegram or Slack.

- Switch on *Allow remote control from chat*, then a channel. **Telegram:** create a bot with @BotFather, paste its token, list the numeric user ids that may command it (message @userinfobot to find yours). **Slack:** create a Slack app with Socket Mode on, give the bot the `chat:write` scope (plus `app_mentions:read` for `app_mention` and `im:history` for `message.im`; `channels:history` if you use `message.channels`) and the app-level token the `connections:write` scope, subscribe to bot events (`app_mention` for channels, `message.im` for direct messages), then paste its bot token (`xoxb-…`) and app-level token (`xapp-…`) and list the member ids (`U…`) that may command it. One message that arrives as more than one event (for example both `app_mention` and `message.channels`) is run once. Tokens are kept in the credential store and never shown again.
- The bot answers only the listed people; anyone else is ignored without a reply. Commands: `/jobs`, `/run <job>`, `/stop <job>`, `/status`, `/help`. Jobs are matched by their exact name or id (never a partial name); if a backup, a sync pair and a rule share a name, say which with `backup:Name`, `sync:Name` or `rule:Name`. Nothing else is understood — the bot cannot run a program or touch a path.
- Every accepted command is written to the tamper-evident audit log before it runs; a command whose audit entry cannot be written is not run. Each person is limited to a number of commands per minute (default 10). Commands sent while the app was not listening are never run late: on start the Telegram bot first skips the waiting messages (and keeps retrying until it can, so an offline start never replays them).

## Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| Ctrl/Cmd+K | Open Command Palette |
| Ctrl/Cmd+Z | Undo last operation |
| Ctrl/Cmd+A | Select all |
| Ctrl/Cmd+N | New file |
| Ctrl/Cmd+Shift+N | New folder |
| F2 | Rename selected |
| Delete/Backspace | Delete selected |
| Ctrl/Cmd+C | Copy |
| Ctrl/Cmd+X | Cut |
| Ctrl/Cmd+V | Paste |
| Ctrl/Cmd++ | Larger text |
| Ctrl/Cmd+- | Smaller text |
| Ctrl/Cmd+0 | Default text size |
| Space | Quick preview (when preview pane is open) |

---

## Themes & Accessibility

### Themes

- **System** - Follow OS light/dark preference (default)
- **Light** - Force light theme
- **Dark** - Force dark theme
- **High Contrast** - Optimized for visibility, meets WCAG contrast requirements

Theme switching is instant with no restart required. The theme applies immediately via CSS custom properties.

### Appearance settings

**Settings → Appearance** collects the display preferences. They are stored with the other window preferences on this computer and apply at once:

- **Theme** - the same choices as the toolbar theme switcher.
- **Text size** - Small (87.5%), Default, Large (112.5%) or Larger (125%). It scales the root font size, so text, spacing and controls sized in `rem` grow together. Shortcuts: **Ctrl/Cmd + +** (larger), **Ctrl/Cmd + -** (smaller), **Ctrl/Cmd + 0** (default); they are also in the command palette and the `?` cheat sheet. Ctrl/Cmd + = (without Shift) stays *Compare files*. The webview's own zoom keys are off in the desktop app, so these replace them.
- **Size units** - *Decimal* (default; 1 KB = 1,000 bytes, as macOS and disk makers count) or *Binary* (1 KiB = 1,024 bytes, labelled KiB, MiB, GiB). Every size in the app follows it: file lists, transfers, sync, backups, mounts and drives. Sizes already on screen in a panel update the next time it redraws.
- **Backup times** - show the times of backup versions, runs and files in this computer's time zone (default) or in UTC (marked "UTC"). Schedules keep their own time zone; see [backup-policies-and-schedules.md](backup-policies-and-schedules.md#time-zone).

### Accessibility

- Full keyboard navigation with visible focus indicators
- ARIA labels on all interactive elements
- Screen reader support with `role="application"`, `aria-current`, and `aria-modal`
- Focusable list items with arrow key navigation
- Reduced motion support
- Minimum touch target size of 44x44px in Simple mode
