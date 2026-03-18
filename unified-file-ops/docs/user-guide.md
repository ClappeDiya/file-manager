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

## Peer-to-Peer Transfers

### Discovery

UFOP discovers other UFOP instances on the local network using mDNS/Bonjour. Discovered peers appear in the sidebar with their hostname and display name.

### Trust Levels

- **Untrusted** - Requires approval for each transfer
- **Trusted** - Automatically accept transfers
- **Blocked** - Reject all requests

### Transferring to Peers

1. Select files
2. Choose "Send to Another Computer" or drag to a peer in the sidebar
3. The recipient approves (if not trusted)
4. Transfer proceeds over a TLS-encrypted connection

### Manual Connection

If peers are not auto-discovered, enter an IP address and port to connect manually.

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
- **Documents** - Rendered preview where supported

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

### Chat

Ask questions about files, get help with operations, or troubleshoot errors. The AI provides contextual assistance based on what you're doing in the app.

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
| Space | Quick preview (when preview pane is open) |

---

## Themes & Accessibility

### Themes

- **System** - Follow OS light/dark preference (default)
- **Light** - Force light theme
- **Dark** - Force dark theme
- **High Contrast** - Optimized for visibility, meets WCAG contrast requirements

Theme switching is instant with no restart required. The theme applies immediately via CSS custom properties.

### Accessibility

- Full keyboard navigation with visible focus indicators
- ARIA labels on all interactive elements
- Screen reader support with `role="application"`, `aria-current`, and `aria-modal`
- Focusable list items with arrow key navigation
- Reduced motion support
- Minimum touch target size of 44x44px in Simple mode
