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
8. [Folder Compare](#folder-compare)
9. [Cloud & Server Connections](#cloud--server-connections)
10. [Peer-to-Peer Transfers](#peer-to-peer-transfers)
11. [Archive Operations](#archive-operations)
12. [Batch Rename](#batch-rename)
13. [File Preview](#file-preview)
14. [Integrity Tools](#integrity-tools)
15. [Terminal](#terminal)
16. [AI Assistant](#ai-assistant)
17. [Encrypted Vaults](#encrypted-vaults)
18. [Cross-Platform Compatibility](#cross-platform-compatibility)
19. [Keyboard Shortcuts](#keyboard-shortcuts)
20. [Themes & Accessibility](#themes--accessibility)

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
- **Parent folder** - The ↑ button next to Back/Forward, **Alt+↑** (Windows, Linux), **⌘↑** (macOS) or **Backspace** (when no text field has focus) opens the folder one level up, locally and on a connection. It stops at the top: `/`, a drive (`C:\`), a network share (`\\server\share`) or the connection's root. Also in the command palette: *Go to Parent Folder*.
- **Tab Bar** - Open multiple directories in tabs (Advanced mode). Tabs can be pinned, reordered, and closed.
- **Favorites** - Star any folder for quick access from the sidebar
- **Cloud folders** - Folders that a cloud service's own app keeps in sync on this computer are listed in the sidebar and open like any local folder: Dropbox (personal and business, also when moved), OneDrive (personal and work accounts), Google Drive for desktop, iCloud Drive, Box, pCloud Drive, MEGA, Nextcloud and ownCloud (every sync folder in the client's settings), Synology Drive and Proton Drive. They are found from the apps' own settings and default folders (on macOS, `~/Library/CloudStorage`); UFOP does not contact the services. The list refreshes every few minutes and the section is hidden when none is found.
- **Recent Locations** - The last 20 visited paths are remembered

### Several windows

**New Window** (**Ctrl+N**, **⌘N** on macOS; command palette; tray menu) opens another window at the active pane's folder, and **Open in New Window** in a folder's context menu opens that folder. Each window has its own panes, tabs and layout; favorites, recent places, saved searches, workspaces and every setting are shared, and a change made in one window (theme, accent colour, language, a new favorite) shows in the others. The first window keeps its layout for the next start. Closing another window just closes it; *Keep running in the tray* applies to the first window, and the app stops (running backups end at a safe point) only when its last window closes.

### Selection

- Click to select a single file
- Ctrl/Cmd+Click to toggle individual items
- Shift+Click to select a range
- Ctrl/Cmd+A to select all

### Dual Pane (Advanced Mode)

In Advanced mode, the workspace splits into two panes. Each pane has its own tabs, view mode, sorting, and filter settings. The split ratio is adjustable between 20%-80%.

### Address bar

Click the empty part of the breadcrumb bar, press **Ctrl+L** (⌘L), or choose **Go to Address…** in the command palette to type where to go:

- a folder on this computer (`/home/me/Documents`, `C:\Users\me`);
- a saved connection as `<connection name>:<path>` (`Work Drive:/Reports`), or `conn://<connection id>/<path>`. Folder names work for every connection: on Google Drive, which addresses folders by id, `Work Drive:/Reports/Q1` is looked up folder by folder and the pane opens the folder found (its address then shows the ids).

Suggestions list the folders of the folder you are typing in (as you type) and places you visited recently. **Tab** completes the highlighted suggestion, **Enter** goes there, **Esc** cancels. An address that does not exist, or names a file or an unknown connection, is shown in red and the field stays open.

### Cloud and server folders in the panes

Both panes can show a saved connection (SFTP, S3, Google Drive, Dropbox, OneDrive, WebDAV, rclone remotes…) next to, or instead of, a folder on this computer:

- **Open one**: click it under **Connections** in the sidebar (opens in the active pane), **Open in pane** in the Connection Manager (also for the folder being browsed there), **Open <name> in Pane** in the command palette, or type its address.
- Breadcrumbs start at the connection (cloud icon and name); tabs, back/forward and the open tabs at the next start keep connection folders like any other.
- **Copy and move between any two places** with F5 / F6, Copy/Cut + Paste, or drag-and-drop: this computer → connection uploads, connection → this computer downloads, and between connections the files are copied on the server when both ends are the same connection and it can (S3, WebDAV, Dropbox, Google Drive, rclone), else — or when that server-side copy fails, e.g. an S3 object over 5 GB — downloaded and uploaded through a temporary file. Every file is a job in the transfer queue (progress, pause/cancel, history); at most 4 files per connection are transferred at once (transfer setting `max_location_jobs_per_connection`), the others wait *Queued*. **Cancel** stops a file at once and removes what it had written; **Pause** stops it the same way and starts that file over on **Resume**. A file that fails part-way is removed too, so **Retry** is not refused by a half-written copy. Google Docs, Sheets, Slides and Drawings copied out of Drive are exported in the connection's format and named with its extension (`Plan.docx`); a *move* out of Drive copies the export and keeps the original document (you are told so). Within Drive they are copied as documents. Folders are copied with everything in them (also into Google Drive, where each new folder's contents go into it by its id). Names already taken at the destination get ` (copy)` (a name differing only by upper/lower case counts as taken, since many servers treat them as one file); nothing is overwritten, also not a file saved at the destination while a download runs. Moving items into the folder they are already in does nothing. Files downloaded to this computer, including everything inside downloaded folders, are checked against the **filename policies** first; if any is blocked, nothing is downloaded. A move within one connection is a server-side rename where possible; other moves delete each original once its copy has finished, then the emptied folders (if the app is closed meanwhile, or a file still fails once its retries are used up, the originals of the unfinished files and their folders are kept).
- From a connection with no other pane to copy to (single-pane view, or the other pane shows the same folder), F5/F6 use the **default download folder** (Settings › General), else ask for a folder.
- **Rename** (F2; a name already used in the folder is refused, nothing is replaced), **New folder**, **Delete** (on the server: there is no local Trash; Google Drive, OneDrive and Dropbox keep deleted items in their own trash), **Calculate size**, **Share Link…** (providers with links) and **Get Info** work on connection items. The status bar shows the account's storage.
- Opening a connection file downloads a copy (up to 25 MB, checked while it downloads) and shows it in the preview pane, or in its default app; changes to that copy are not uploaded. A program or script (`.exe`, `.bat`, `.sh`, `.lnk`, `.desktop`…) asks first, since it would run on this computer. Copies older than a week, and the oldest beyond 1 GB, are removed at startup.
- Actions that only work on this computer's files (duplicate, compress, extract, convert, GPG, integrity tools, terminal, batch rename, file history, thumbnails, subfolder/content search…) stay in the menus but are disabled, with the reason in their tooltip.
- A connection that cannot be listed (offline, signed out, credentials rejected) shows why, with **Reconnect** (which also clears a *Credentials rejected* flag) and **Connections…**.
- Undo covers operations on this computer's files only.

### Drag-and-drop

Dropping files in the same place (this computer, or the same connection) moves them (hold **Alt** to copy); dropping onto another connection, or between this computer and a connection, copies them. **Settings › General › Ask before drag-and-drop copies or moves** (off by default) shows what a drop will do first — measured with everything inside dropped folders, and with the Safety Interlock's warning for unusually large moves — and runs it only when you confirm.

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

- **Date chips:** *Today*, *Yesterday*, *Last 7 days*, *Last 30 days* or *Custom* (two dates) narrow the list, the count and the CSV export alike. The same chips are on Sync history and Backup run history. Times are shown in your local time.
- **Open source / Open destination:** a row's buttons reveal a local folder in the file manager, or open a remote location in the connection browser.
- **Upload / Download / Sync chips** in the Transfers panel filter the queue by direction (a path like `s3://bucket/key` is the remote side). Each running sync shows an expandable list of its files (current, finished, and any that failed), capped so a huge sync stays light.
- **Failed transfers survive a restart.** A failed transfer is saved at once, comes back after restart still marked *failed* (it is never retried by itself) and can be retried with the usual button. Failed entries are cleaned up after a time and size limit (default 30 days, 4096 KB; Transfer history → *Failed transfers stay available…*), which also run with *Clean up*.

### Run history detail

Sync, backup and Quickflow runs show **who started them**: *Manual*, *Scheduled*, *Watcher*, *API* or *Automation*. A sync run lists its **changed files** (copied, updated, deleted, moved, failed, conflict), up to 10,000 per run with a note when the list was cut short, and a backup job with several destinations keeps a **result row per destination** for every run, not just the last one.

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

### Two-way sync and deletions

A two-way pair remembers what both folders looked like after its last successful run (its *sync history*). Each run compares both sides with it:

- A file deleted on one side and unchanged on the other is **deleted on the other side too** (with a rollback copy). Renaming a file is a delete plus an add.
- A file changed on one side is copied to the other, in either direction. A file edited while a run is copying it counts as changed on the next run, so the edit is copied then.
- A folder deleted on one side is removed on the other once its files have been deleted there (a folder that still holds other files stays).
- A file changed on **both** sides is a conflict and goes through the pair's conflict policy, unless both sides now hold the same bytes (checked by content hash), which counts as in sync. *Destination wins* and *Keep both* leave both folders identical (keep both: the destination's version under the name, the source's as `name (conflict DATE).ext`, on both sides).
- The **first run** (no history yet) only combines both folders; it never deletes. The preview says so.
- A run that fails, is cancelled or stops at the end of its run hours keeps the previous history, so nothing it left half done is mistaken for a deletion. Until a run finishes cleanly, deletions of files created since the last clean run are not tracked (such a file comes back from the other side); the preview says so after a run with errors.
- **Move files** (/MOV, /MOVE) is not available for two-way pairs: a moved file would be copied back to the source on the next run. Two-way pairs do not take it from the global defaults either. A two-way pair saved with it by an older version keeps working safely (a moved file leaves the sync history, so it is copied back rather than deleted), and its preview says to turn it off.
- **Names that differ only in letter case** (`Report.txt` on one side, `report.txt` on the other, e.g. after a rename that only changed the case) are skipped with a reason and a preview note, and their sync history is forgotten: on drives that ignore case (the macOS and Windows defaults) they are one file, so copying or deleting either name could remove it. Rename one side to match and the next run settles them. As a second safeguard, a delete only goes through when the folder lists the file under exactly the planned name.
- **Safety stop**: a run that would delete more than 50% of the synced files on one side (more than two files), or everything because one folder is empty or missing, is refused — usually an unplugged drive or an unmounted share. Change the limit in *Cleanup & moving* (0 turns it off) or reset the history.

With **Destination state: Blind upload** the destination is never read, so a two-way pair only copies source files to it and never deletes anything (no history is used or recorded). With **Trust the inventory** a two-way pair still scans the destination every run, because the saved inventory cannot show changes made there. Changing a pair's mode or folders forgets its history. In the Advanced view each two-way pair shows its history size and a **Reset history** button.

### Edit, duplicate, export and import

- **Edit** opens the pair in the same form used to create it.
- **Duplicate** creates "*name* (copy)" with the same settings, no run history and its schedule (or watcher) off: a scheduled or watched pair's copy keeps its trigger (the cron expression included) but starts disabled; turn it on with **Enabled** in Edit once it points at its own folders. Backup jobs have **Duplicate** too: the copy's schedules (including replica timers, and a schedule or maintenance plan it would inherit from a policy template) are off, stored credentials are copied rather than shared, and it opens in the editor so you can give it its own destination.
- **Export** / **Import** (Advanced view) save one pair as a `ufop-sync-pair` JSON file and load it on another computer. If a folder is missing there, or a folder is on a saved connection's mount that the computer does not have, the import says so and the pair stays disabled until you fix it. A pair that runs on a schedule or when files change also arrives turned off, so it cannot start syncing (or mirror-deleting) before you have checked its folders; turn it on in **Edit**.

### Filters, rules and presets

Besides the include/exclude lists, a pair (and a backup job) can have **ordered rules**: the first rule that matches decides, so `+ keep.log` above `- *.log` keeps that one log file. Rule kinds: wildcard, exact path, regex, a group (temporary files, caches, OS metadata, build output, images, video, audio, documents, archives) and **rclone glob** (`*` stays in one folder, `**` crosses folders, a leading `/` anchors at the top of the pair or of each backup source folder, a trailing `/` matches a folder and everything in it, `{jpg,png}` alternatives). As in rclone, an rclone pattern without a trailing `/` or `**` names files only: `- *` never hides a folder, so `+ *.jpg` followed by `- *` keeps JPEGs at any depth, and `+ /Photos/**` followed by `- *` keeps only the `Photos` folder of each source. The other rule kinds in a backup job match the full path on disk (`/home/me/cache/`, `*/cache/`). A folder rule covers everything inside that folder; in a sync pair, paths are matched from the pair's top folder (so `cache/` means the `cache` folder at the top of the pair, and `/proc/`-style anchored group entries refer to the pair's top as well).

- **Import rules from file…** reads an rclone `--filter-from` file: one `+ pattern` or `- pattern` per line, `#` and `;` comments, `!` clears the rules read so far. On the command line, `ufop backup run` / `estimate` take rclone rules as `--filter "+ /Photos/**"` (repeatable, applied in order after `--include` / `--exclude`), which is also how a backup job's *CLI command* writes them.
- **Saved presets**: *Save as preset…* stores the current rules under a name; *Apply saved preset* adds a preset's rules to a pair or job (both views). In the Standard view a sync pair lists its rules read-only under the picker, each with a remove button and *Clear all rules*, so an applied preset can be undone without switching views. Delete presets in the Advanced view.
- The quick filter presets of the sync form now include images, video, audio, documents and archives.

### Comparing files

In the Advanced view, *When to overwrite › Compare files by* chooses **size and date** (default), **checksum** (xxHash3; reads every file whose size matches, finds changes even when the date did not change and ignores files that were only touched), or **size only**. When checksums are not available the preview notes that size and date were used. Checksums are computed by reading the files through their local or mounted path; hashes stored by a cloud provider are not used yet. The **Ignore existing** preset (Advanced) copies only files the destination does not have.

### Versioned backup: versions to keep

Versioned-backup pairs keep every stored version unless *Versions to keep of each file* (Advanced) is set; then the oldest versions beyond that number are deleted after each run. Unchanged files are no longer stored again on every run; if the newest stored version of a file is removed from the destination, the next run stores the file again. Versions stored by earlier app versions (numbered `name.v1.ext` files without a version list) are taken over on the first run, so they count towards that number and continue the numbering; this works for the default name and any template built only from `{name}`, `{ext}`, `{filename}` and `{num}`.

### Interrupted runs

Each finished step of a run is recorded as it happens. If the app quits, crashes or the run is cancelled, the next run skips the steps that were already done (for files that have not changed since, and whose copy is still in place at the other side) and its report says *Resumed from interruption*. Editing the pair's folders, mode, filter or options, or deleting the pair, discards that record; versions a versioned backup stored before a crash are added to its version list on the next run.

### Global sync defaults and bandwidth timetable

Settings › Advanced has:

- **Sync defaults**: copy options new pairs start from. A pair created afterwards stores only the options it changes and follows the defaults for the rest; pairs created before keep all their own options.
- **Bandwidth timetable**: windows (weekdays, from–to) with upload and download limits. While a window is active its upload limit is shared by file transfers, sync copies and backups together (transfers and sync copies count as uploads), its download limit applies to backup restores, and each replaces the backup machine-wide limit for its direction. A window's 0 sets no limit of its own: the machine-wide limit for that direction stays in force (a download-only window does not lift the upload cap). The backup machine-wide limits themselves still apply to backups and restores only. A window that ends before it starts runs past midnight; for each direction, the first active window that limits it wins (a download-only window listed first does not hide a later upload limit).

### Rollback

If a sync produced unwanted results, use the Rollback feature to restore files from the pre-sync backup (not while the pair is syncing: wait for the run to finish or cancel it). Two-way runs keep rollback copies of source files they changed or deleted too, and Rollback puts each back on its own side, with its original modification time. Rolling back a two-way run also forgets the restored files' sync history: if a restored file now differs from the other side, the next run handles it as a conflict under the pair's conflict policy (with *Keep both*, both versions are kept) rather than copying either version over the other.

### Reports

Every sync run generates a report with:

- Files added, modified, deleted, skipped
- Conflicts resolved and errors
- Bytes transferred and duration
- Health status

Reports can be exported as CSV or JSON.

---

## Folder Compare

Compare two folders recursively and reconcile them by hand — without setting up a sync pair. One side is always a folder on this computer; the other is another local folder or a folder on a saved connection (SFTP, S3, WebDAV, a cloud drive such as Google Drive, an rclone remote…). Comparing two connections with each other is not offered yet.

**Opening it**

- **Compare** in the pane toolbar (dual-pane view) compares the left and right panes' folders. The same is in the command palette (*Compare Panes*), and as the optional *Compare Folders* toolbar button (Customize Toolbar).
- *Compare Folder With…* (command palette) compares the active pane's folder with a folder you pick.
- In **Connections**, while browsing a remote folder, **Compare with…** compares it with the left or right pane's folder, or with a local folder you choose.

**Options**: *Compare by* size and modification time (default), size only, or checksum (xxHash3 of the content; files on a connection are compared by size and time and the window says so); *Folder depth* (0 = all levels; deeper folders show "contents not compared"); *Include* / *Exclude* patterns and **Filter rules** — the same filter engine, rule kinds and saved presets as sync pairs; *Allow 2 s time difference* for FAT drives and servers that store coarse times (on by default when a side is a connection). The comparison lists both sides at once (several folders at a time), shows progress, and **Cancel** stops it.

**Results**: every file and folder with its size and date on each side and a status — **Left only**, **Right only**, **Same**, **Different** (hover for why: size, time or content) or **Error** (could not be read; the reason is shown). Everything inside a folder that one side could not read is an error too (what that side has there is unknown), and so is a name that is not a plain file name (for example a server file called `..\..\notes.txt`, shown with its slashes replaced); errors are never copied or deleted. A folder's status comes from what is inside it, and its row counts the differences below it; folders that match start collapsed. The status chips at the top filter the list (several chips show any of them) and show how many entries each status has.

*Advanced view* adds **Next difference**, **Largest by size** / **Largest by file count** (jumps to the folder with the most bytes or files that differ, opening the folders above it) and sorting by **size difference**.

**Acting on differences**: tick rows (Shift-click ticks a range; a ticked folder stands for everything in it), then:

- **Copy → right** / **Copy ← left** copies the ticked entries that are on that side and missing or different on the other, creating folders as needed. Entries that are already the same are never copied, and neither are folders whose contents were not compared (below the depth limit, or linked folders): compare with a deeper limit to copy them. A file that was not on the other side when compared but is there now is not overwritten (local folders and connections alike); compare again. Local copies run as verified transfer-queue jobs and keep the file's date; copies to or from a connection go through the connection and appear in the transfer queue too, where **Cancel** stops them (they cannot be paused or retried from the queue; run the action again instead). A local file that is replaced is moved to the Trash first (on Windows it is deleted: the Trash is not available there yet). A file on a connection that is replaced is overwritten by the upload and its previous version is not kept, so the window asks before such a copy.
- **Delete on left** / **Delete on right** (asks first) deletes the ticked files on that side — to the Trash for a local folder (deleted for good on Windows, where the Trash is not available yet); on a connection the server deletes them, which on most kinds of connection cannot be undone (Google Drive moves them to its own trash) — and then the ticked folders once they are empty (a folder that still holds files the comparison filtered out is kept, and the window says so). *Delete permanently* skips the Trash for deletes and replaced files; changing it while a delete waits for confirmation plans and checks that delete again.

Every action is checked by the Safety Interlock (an unusually large delete or overwrite asks for confirmation), recorded in the Activity Timeline, and only the entries it touched are compared again. New local copies (and folders created for them) can be undone with **Undo copies** or from the timeline; replaced and deleted local files are in the Trash (except on Windows). Copies, replacements and deletes on a connection are recorded but cannot be undone from the app. Uploads to some connections get the upload time as their date: if they still show as *Different*, compare by size only or checksum. A local copy is written to a hidden `.<name>.ufop-compare-…` file next to its destination and renamed into place when complete; if the app closes during the copy, that job is not resumed and the hidden file is removed the next time a copy goes to that folder.

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
| **Backblaze B2** | Native API (application key ID + key) with large file uploads. S3 compatibility mode available. |
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

### Signing in to Google Drive, Dropbox and OneDrive

Add the connection from **Connections → + Cloud** (a guided wizard) or with **+ New** and the provider as protocol. After it is saved, **Sign in with …** opens the provider's page in your browser; when you approve, the browser returns to UFOP (on `http://localhost:9876`, `9877` or `9878` for Google Drive, Dropbox and OneDrive) and the connection is signed in. No password is stored: the access the provider grants (OAuth tokens) is kept in the OS keychain and renewed automatically when it expires. Use **Sign in** on the connection later to switch accounts. A connection without a stored sign-in reads *Not signed in*; **Sign out** forgets its sign-in (and disconnects it if it is open).

- **Browser on another device**: open the sign-in page there and paste the code (or the whole address the browser was sent to) into *Paste the code*.
- **Computer without a browser** (*Sign in on a computer without a browser*): paste tokens (an access token, or JSON with a refresh token and its client ID), or an **AuthID** from your self-hosted sign-in broker (`ufop-server --oauth-config`, open `/oauth/<provider>/start` there). The credential is checked by connecting before it is saved.
- **Built-in app**: a build of UFOP can include its own OAuth app (`UFOP_GOOGLE_DRIVE_CLIENT_ID`, `UFOP_DROPBOX_CLIENT_ID`, `UFOP_ONEDRIVE_CLIENT_ID`, with `_CLIENT_SECRET` for Google; set when building or running). A build without one says so in the sign-in screen. In **Advanced** view you can then use **your own OAuth app**: enter its client ID (and, for Google, its client secret) and register the redirect URI shown (`http://localhost:<port>/callback`).

**Where the connection opens** (saved on the connection):

- **Google Drive**: My Drive, Shared with me, or one shared drive (choose it with *Load*). Google Docs, Sheets, Slides and Drawings are exported when downloaded, by default as docx, xlsx, pptx and pdf; Advanced view lets you pick other formats (for example pdf or csv).
- **OneDrive**: a personal or a work/school account; for work accounts you can open a **SharePoint document library** instead of your OneDrive (search the site, then choose the library).
- **Dropbox**: your own folder, the **team space** or a **team folder** (*Load team spaces*).

### Connection settings

- **SFTP**: the password, or the private key path (*Password*) and the key's **passphrase**. A key generated in the form with a passphrase fills it in. When editing, a blank passphrase keeps the saved one; tick *Remove saved passphrase* to forget it.
- **WebDAV**: sign in with a user name and password, a **bearer token** (app passwords and tokens), or none; choose **Nextcloud** or **ownCloud** for their chunked uploads.
- **SMB**: the Windows **domain** (optional). On Linux the password reaches `mount.cifs` through the environment, not the command line. On macOS (`mount_smbfs`) and Windows (`net use`) those tools only take it as a command argument, so while the mount command runs (a moment) other programs of the same user can see it in the process list; avoid mounting with a password on a shared machine, or use a Kerberos / saved Windows credential (leave the password empty).
- **Backblaze B2**: the **application key ID** and **application key**.

### Connection Management

- **Save** - Store connection profiles with names and groups
- **Test** - Verify connectivity before saving
- **Groups** - Organize connections into named groups
- **Star** - Starred connections (☆ next to the name) are listed first
- **Show password** - The eye button in a password field shows what you typed
- **Import/Export** - Share connection profiles between machines
- **Credential Storage** - Passwords and keys stored in the OS keychain (macOS Keychain, Windows Credential Manager, Linux Secret Service)
- **Rejected credentials** - When a server refuses the sign-in (wrong password, expired or revoked access), the connection is marked *Credentials rejected* and automatic reconnects (backups, automation, auto-mount) skip it instead of retrying the same credentials, which could lock the account. Edit the connection with the current credentials, or click **Retry**. Network errors are still retried.
- **Default folders** - Settings › General: the default download folder (where the download folder picker opens) and the default upload source folder (where the upload file picker opens).
- **Open in pane** - Browse a connection (or the folder open in the panel) in the active file pane; see *Cloud and server folders in the panes*.
- **Folder picker** - Sync pairs, backup sources and destinations (including a folder on a backup connection) and the mount dialog's subfolder have **Pick…**: a folder tree of this computer and your connections, with **New folder**. The system **Browse…** button is still there for local folders.
- **Storage and folder size** - When connected, the panel shows the account's storage (Google Drive, OneDrive, Dropbox, rclone remotes). Right-click a remote folder → **Calculate size** to add up everything below it (rclone asks the server; other connections walk the folders, eight at a time); *Stop* ends it early with partial figures.

### rclone remotes (70+ more providers)

Any storage rclone supports (Box, pCloud, MEGA, Proton Drive, iCloud Drive, Google Photos, Yandex Disk, HiDrive, Koofr, Oracle Object Storage, Gofile, HTTP, put.io, Zoho WorkDrive, ShareFile and the rest) can be used through an **rclone connection**. The app runs the rclone program on your computer.

- **Settings → rclone** shows the rclone in use (path, where it came from, version) and the config file. Leave the program blank to use `UFOP_RCLONE` or the `rclone` on your `PATH`, or choose one (**Browse…**, or **Detect** to list the copies on this computer with their versions). Leave **rclone.conf** blank for rclone's own default, or choose a file; it is passed to every rclone call (connections, backups, mounts, the remote manager). An rclone older than the minimum version is flagged. *Advanced*: **Install rclone** downloads the pinned release for this computer from downloads.rclone.org, checks it against the release's `SHA256SUMS` (a mismatch installs nothing) and installs it into the app's data folder, then uses it; **Update** appears when the app pins a newer release, and **Use managed copy** switches back to it after you chose another rclone. An rclone you installed yourself is never replaced. A backup job that sets `RCLONE_CONFIG` in its own environment keeps that config file rather than the one chosen here.
- **Connections → rclone remotes** (also *+ Cloud → More … via rclone* and the palette's *rclone Remotes…*) lists your remotes with their type. **New remote** offers a *Popular* list and every backend the installed rclone has; the form is generated from rclone's own description of the backend: required and everyday settings in Standard mode, the backend's advanced options in Advanced mode. Passwords are obscured by rclone when saved (*Advanced*: tick *already obscured* if you paste output of `rclone obscure`). Providers that sign in with OAuth (Box, Google Photos, pCloud, …) ask **Sign in with browser**: your browser opens rclone's sign-in page (a link is shown too) and the remote is finished when you approve; **Cancel sign-in** stops it. *Answer rclone's questions manually* goes through rclone's questions one by one instead (for example to paste a token from `rclone authorize` on another computer). Remotes can be **tested** (lists the top folder), **edited** (stored secrets are never shown; leave them blank to keep them) and **deleted** (from rclone.conf only; nothing on the storage is touched). **Add as connection** opens a new rclone connection for the remote. A popular provider the installed rclone does not have is shown greyed out: update rclone.
- **Encrypted (crypt) remotes** (*Advanced*): browsing an rclone crypt remote shows a banner, *Encrypted remote: names and contents are decrypted locally; underlying remote: …*, with **Open underlying remote** (browse the encrypted data as stored) and **Encrypt / decrypt names…**, which translates file names offline with the remote's keys (`rclone cryptdecode`, and `--reverse` to encrypt); also from **Names…** in the remote list.
- **About** (the status bar's settings popover) shows the rclone version and path, or *not installed*; diagnostics bundles include it.

rclone receives option values on its command line while a remote is created or changed (rclone has no other non-interactive way), so on a shared computer other users could see a password in the process list for that moment.

### Guided Wizards

Step-by-step wizards do the real work, each through the same commands as the full screens, and say what they did in the Activity feed. Simple mode lists them; in every mode the command palette has them all (search for the name, or "guided"):

- **Connect OneDrive / Google Drive / Dropbox** (save, sign in, choose where it opens, check it by listing it) and **Google Cloud Storage** (HMAC keys, checked by listing the buckets).
- **Connect to Windows Share (SMB)** and **Connect via SFTP** - *Test Connection* saves the connection, signs in and lists the folder, so a wrong password or share name shows here; the connection is then in the Connection Manager. SFTP with *SSH Key* uses the keys your SSH agent holds.
- **Set Up Local Sync** and **Create Sync Pair** - *Test Folders* checks both folders exist; the pair (one-way, two-way or mirror; manual, hourly, daily at 02:00 or real-time) appears in the Sync panel.
- **Transfer Files** - copies or moves a file or folder (also to or from a connection, through the transfer queue); *Verify files after transfer* compares every copied file with its original by checksum.
- **Resolve Interrupted Operation** - lists the paused and failed transfers; resume them (failed ones are retried from the saved progress) or cancel them.
- **Fix File Name Issues** - checks a folder against where it is going (Windows, FAT32, macOS, OneDrive, Google Drive, Dropbox, S3, Backblaze B2…), lists the names that place cannot store with the suggested names, and renames the ones that can be fixed. **Restore Original Names** lists those renames and gives the files their original names back.
- **Export Report** - saves the transfer history or the activity log as CSV or JSON, or the security audit log as signed JSON lines, where you choose.
- **Send to Another Computer** and **Move to a New Computer** - pick a computer on this network running UFOP and the folders to send (or your Documents, Desktop, Pictures, Music, Videos, Downloads); the other computer approves the transfer.
- **Copy to External Drive** - copies a folder onto an external drive after checking there is room. **Migrate Drive** copies everything from one drive into a folder of its own on another (nothing there is replaced) and, unless turned off, checks every file by checksum.
- **Back Up Folder** - creates a backup job (encrypted with a passphrase unless you choose otherwise; now only, daily or weekly) and runs the first backup.

---

### Mounting Drives

*Mount drive* turns a saved connection into a folder (SMB, NFS and SFTP through the OS; cloud storage through rclone + FUSE). Cloud drives have cache options:

- **Cache mode** — *Off* (always from the server), *Minimal* (open files only), *Writes* (default: uploads finish in the background), *Full* (whole files are kept, so they stay readable offline).
- **Largest cache** and **Keep cached files** limit the space and age of the local copies; **Read-only** refuses changes.
- With the full cache, **Make available offline** downloads a folder (or the whole drive) into the cache; the cache size shows per drive, and **Clear cache** frees it once the drive is unmounted.
- Your organization's **mount profile** (from the management console) may allow only some kinds of drives, require read-only mounts or a cache mode, cap the cache, or require mount points inside a folder; the dialog shows it and locks the options it forces.

**Where to find it.** In the Connection Manager, **Mount** on a connection row opens the mount dialog with that connection selected. The command palette has *Mount Drive…* and *Show Mounts*, and the toolbar customizer has a **Mounts** item. The tray icon has a **Mounts** submenu: a check mark shows which saved mounts are mounted, a click mounts or unmounts one, and *Unmount all* unmounts everything.

**The Mounts list** shows every saved and running mount with its state: *Mounted*, *Unmounted* or *Error* (with the reason). A mounted drive has **Open** (shows it in the file manager) and **Unmount**; the others have **Mount**. **Edit** opens the dialog filled in with the settings you saved, not the limits your organization applied on top (unmount the drive first; the new settings apply to the next mount), **Remove** deletes the saved mount (after you confirm; a mounted drive is unmounted first) and **Unmount all** unmounts every mounted drive. A mount marked *Auto-mount* is mounted again when the app starts. A drive that stops by itself (rclone ends, or it is unmounted outside the app) shows as an error (on Windows this is noticed for drives mounted as a drive letter, not for ones mounted in a folder). In Standard mode, a saved mount that uses a drive letter shows it with **Use a folder instead**; the letter picker itself is in Advanced mode.

**In the dialog.** *Subfolder* mounts one folder of the connection (for example `photos/2024` inside the connection's path; `..` is not allowed) instead of everything. *Open the drive after mounting* opens the folder in your file manager once it is mounted (not when the app mounts it at start-up).

**Advanced options** (Advanced mode, cloud drives; the organization's mount profile still decides what it fixes):

- **Drive letter** (Windows): mount as `E:` instead of a folder; only free letters from D to Z are offered. Not possible when the profile requires mount points inside a folder.
- **Volume label** (Windows and macOS), **Show as a network drive** (Windows), **Let other users see the drive** (macOS and Linux; needs `user_allow_other` in `/etc/fuse.conf`).
- **File permissions** and **Folder permissions** as octal numbers (for example `0644`, `0755`); **Owner** and **Group** ids (macOS and Linux).
- **Cache folder**: keep this drive's cache somewhere else (it is kept in a folder named after the drive, so *Clear cache* never touches other files there).
- **Directory cache**: how long folder listings are trusted (`30s` by default, `5m`, `1h`). Longer is faster but shows changes made elsewhere later.
- **Extra rclone flags**: read and timeout tuning such as `--buffer-size=64M` or `--no-modtime`. Flags that have a field above, or that the profile controls (read-only, cache mode and size), are refused.

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

### Command help

The *Command help* button in the terminal toolbar opens a drawer that lists the `ufop` command-line commands (and the `rclone` commands when rclone is found) with a one-line description. Search it, and click a command to type it at the prompt (it is not run until you press Enter).

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

## Logs, feedback and the local web server

- **Log viewer (Advanced):** toolbar *Log viewer*, Settings → Logging → *Open log viewer*, or the command palette. It reads the app's own log files and filters by level, source, time range and text. *Live* follows new lines, rows can be selected and copied, and *Open folder* shows where the files are (`ufop.<date>.log` for people, `ufop.<date>.jsonl` for tools; the log retention setting prunes both).
- **Send feedback:** Help › *Send feedback* (command palette, or Settings → Logging). Write a message, pick a category and, if you want, tick *Include diagnostics* (off by default; secrets, e-mail addresses and folder names are scrubbed). On a computer registered with a management console it is sent there; otherwise it is saved as a zip you choose, and the dialog says so. Nothing is ever reported as sent when it was not.
- **Local web server (Advanced):** Settings → Advanced → *Local web server* runs the web interface and API of `ufop-server` inside the app. It is off until you press *Start server*, listens on this computer only (port 8200 by default) and will not start until you set a password. *Allow other computers* listens on every interface and switches to HTTPS with a generated certificate. See `docs/server-operations.md`.
- **Saved servers:** the same section lists the remote `ufop-server` addresses the `ufop` command line uses (`ufop/config.toml` in your user config folder, or the file named by `UFOP_CONFIG`). *Test connection* asks the server's `/api/v1/health` and shows its version and the delay. Tokens are never shown; sign in with `ufop server login`.

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
- **Language** - *System default* (the computer's language) or English, Español, Français, Deutsch, Português, 日本語, 简体中文 or 한국어. It applies at once to the menus, the sidebar, the Settings headings, the navigation buttons and the tray menu, and is also offered on the first page of the onboarding wizard. Text that has no translation yet stays in English. A language chosen here is kept even when it is the same as English on a computer set to another language.
- **Accent colour** - Blue (default), Indigo, Violet, Pink, Rose, Orange, Green, Teal or Graphite, or **Custom** for any colour. It recolours buttons, selected items, links, progress bars and focus outlines in the light and dark themes; shades are derived in OKLCH from the chosen colour so text on them stays as readable as on the default blue. High contrast keeps its own colours.
- **Text size** - Small (87.5%), Default, Large (112.5%) or Larger (125%). It scales the root font size, so text, spacing and controls sized in `rem` grow together. Shortcuts: **Ctrl/Cmd + +** (larger), **Ctrl/Cmd + -** (smaller), **Ctrl/Cmd + 0** (default); they are also in the command palette and the `?` cheat sheet. Ctrl/Cmd + = (without Shift) stays *Compare files*. The webview's own zoom keys are off in the desktop app, so these replace them.
- **Size units** - *Decimal* (default; 1 KB = 1,000 bytes, as macOS and disk makers count) or *Binary* (1 KiB = 1,024 bytes, labelled KiB, MiB, GiB). Every size in the app follows it: file lists, transfers, sync, backups, mounts and drives. Sizes already on screen in a panel update the next time it redraws.
- **Backup times** - show the times of backup versions, runs and files in this computer's time zone (default) or in UTC (marked "UTC"). Schedules keep their own time zone; see [backup-policies-and-schedules.md](backup-policies-and-schedules.md#time-zone).

### App lock

**Settings → App Lock** locks UFOP's windows with the master password:

- **Lock UFOP with the master password** - the first time, choose a master password (at least 8 characters; it also protects saved credentials and cannot be recovered). Afterwards every change here asks for it.
- **Lock when UFOP starts** and **Lock when not used** - never, or after 1, 5, 10, 15, 30 or 60 minutes without using any UFOP window.
- **Lock now** - also *Lock UFOP* in the command palette and the tray menu.

While locked, every window shows the lock screen and nothing can be read or changed through the app: the desktop app refuses every request except unlocking. Scheduled backups, syncs, automations and transfers keep running. After too many wrong passwords the lock screen waits a few minutes before accepting another one. Locking, unlocking and wrong passwords are recorded in the security audit log.

### Reset settings

**Settings → Reset** (or *Reset Settings…* in the command palette) puts preferences back to the defaults. Choose what to reset:

- **Appearance and layout** - theme, accent colour, language, text size, units, Simple/Advanced mode, toolbar, sidebar, panels, table columns, drag-and-drop confirmation, default download and upload folders.
- **Startup, tray and notifications** (desktop app) - start at login is turned off, the window opens at start, the tray icon shows, desktop notifications go back to their defaults.
- **Sync and transfer defaults** (desktop app) - global sync defaults, the bandwidth timetable and how long failed transfers are kept.
- **Welcome tour and tips** - shows the welcome wizard and the feature tips again (not chosen unless you tick it).

Connections, sync pairs, backup jobs, mounts, favorites, history, saved passwords and keys are never reset, nor are the app lock, the local web server, chat control, metrics, rclone, proxy and console settings. You are asked to confirm first, and the reset is recorded in the security audit log.

### Startup and tray

**Settings → Startup & Tray** (desktop app; also *Startup & Tray Settings…* in the command palette):

- **Start UFOP when I log in**, and **…in the tray, without opening the window** (on by default, as before).
- **Start minimised** - every start keeps the window in the tray until you open it from the tray menu.
- **Keep running in the tray when the window is closed** - scheduled backups, syncs and automations keep running; quit from the tray menu. The same setting is on the Backups screen.
- **Minimise to the tray** - minimising the window removes it from the taskbar or Dock; open it again from the tray.
- **Show the tray icon** - with the icon hidden nothing can hide the window: closing quits, minimising stays in the taskbar, and *Start minimised* starts minimised in the taskbar. The same applies when the desktop has no tray (some Linux desktops).

### Accessibility

- Full keyboard navigation with visible focus indicators
- ARIA labels on all interactive elements
- Screen reader support with `role="application"`, `aria-current`, and `aria-modal`
- Focusable list items with arrow key navigation
- Reduced motion support
- Minimum touch target size of 44x44px in Simple mode
