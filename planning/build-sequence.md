# Build Sequence - AI Agent Execution Order

**Purpose:** This file tells the Orchestrator Agent what to build and in what order.
**Not sprints.** Not story points. Just dependency-ordered tasks with acceptance criteria.

**Rule:** No task may start until all its dependencies are marked DONE.
**Rule:** Every task must load global-policy.yaml + the listed agent YAMLs.
**Rule:** Every task completion must include the mandatory handoff manifest.

---

## How to Use This File

1. Orchestrator reads this file top-to-bottom
2. For each task: check dependencies are DONE
3. Load the listed agent YAMLs
4. Execute the task against the listed PRD sections
5. Verify all acceptance criteria pass
6. Mark task DONE with evidence
7. Move to next unblocked task

Parallel execution is allowed for tasks at the same level with no cross-dependencies.

---

## Phase 1: Foundation (Must Complete First)

### T-001: Project Scaffold and Build Pipeline
- **Dependencies:** None (start here)
- **PRD Sections:** 11.1, 11.2, 11.5, 23.2
- **Load Agents:** rust-core-architect, ci-devops
- **Risk Class:** R2
- **Work:**
  - Initialize Tauri 2.0 project with Rust backend
  - Initialize React + TypeScript + Vite frontend
  - Configure pnpm workspace monorepo structure
  - Set up CI pipeline that builds on macOS, Windows, Linux
  - Configure code signing placeholders for all 3 platforms
  - Set up hot-reload for frontend development
- **Done When:**
  - Project builds successfully on all 3 target platforms
  - CI pipeline produces artifacts for macOS, Windows, Linux
  - Tauri IPC bridge operational with test command/response
  - Hot-reload works in development mode
  - Rust clippy passes with zero warnings

### T-002: Rust Crate Architecture
- **Dependencies:** T-001
- **PRD Sections:** 11.1, 11.5, 11.6, 26.1
- **Load Agents:** rust-core-architect
- **Risk Class:** R2
- **Work:**
  - Create Rust workspace with separate crates: core, fs_engine, transfer_engine, sync_engine, compat_engine, governance, ai_engine, connectors, cli
  - Define shared types crate (FileEntry, TransferJob, SyncPair, CompatResult, AuditEvent)
  - Set up Tauri commands crate that delegates to subsystem crates
  - Configure dependency injection for mock testing
- **Done When:**
  - Each crate compiles independently
  - Shared types compile and are importable by all crates
  - Tauri commands crate compiles and bridges to at least one subsystem
  - cargo clippy and cargo test pass across all crates

### T-003: SQLite Persistence Layer
- **Dependencies:** T-002
- **PRD Sections:** 11.1, 23.1
- **Load Agents:** rust-core-architect
- **Risk Class:** R3
- **Work:**
  - Integrate rusqlite with connection pooling
  - Create initial schema: config, connections, bookmarks, transfer_history, sync_state, compat_mappings, audit_events
  - Set up migration framework (refinery or similar)
  - Store database in OS-appropriate app data directory
  - Ensure all database operations are async-safe
- **Done When:**
  - Schema v1 created with all tables
  - Migrations run successfully
  - Database persists across app restarts
  - Database can be reset without corruption
  - All operations are non-blocking

### T-004: App State Persistence
- **Dependencies:** T-003
- **PRD Sections:** 23.1
- **Load Agents:** rust-core-architect, desktop-ui
- **Risk Class:** R2
- **Work:**
  - Save workspace state on app close (open tabs, pane positions, theme, sidebar state)
  - Restore workspace state on app open within 500ms
  - Handle corrupted state file gracefully (fall back to defaults)
  - Implement crash-safe transfer queue persistence
- **Done When:**
  - App remembers workspace layout across restarts
  - Corrupted state file does not crash the app
  - Interrupted transfers appear in queue on restart with correct status

### T-005: Frontend Design System Setup
- **Dependencies:** T-001
- **PRD Sections:** 11.1, 11.7, 11.9
- **Load Agents:** desktop-ui
- **Risk Class:** R1
- **Work:**
  - Create @ufop/design-tokens package (colors light/dark/high-contrast, spacing, typography, radii, shadows)
  - Create @ufop/ui-components package with shadcn/ui base components
  - Configure Tailwind CSS
  - Set up TanStack Table, TanStack Virtual, TanStack Query
  - Integrate React Aria for accessibility hooks
  - Configure React Router for client-side routing
  - Set up Zustand for client state
- **Done When:**
  - Design tokens render correctly in light and dark mode
  - shadcn/ui components render in Tauri webview
  - TanStack Table renders a test data grid
  - React Aria keyboard navigation works on a test list
  - All libraries compile and bundle via Vite

---

## Phase 2: File Manager Core

### T-006: Dual-Pane Layout
- **Dependencies:** T-004, T-005
- **PRD Sections:** 13.1, 13.9
- **Load Agents:** desktop-ui, rust-core-architect
- **Risk Class:** R1
- **Work:**
  - Build side-by-side pane layout with draggable divider
  - Each pane has independent path navigation (breadcrumb + address bar)
  - Panes resizable from 20% to 80%
  - Single-pane toggle mode
  - Keyboard shortcut to switch focus between panes
- **Done When:**
  - Two panes render with independent navigation
  - Divider dragging works smoothly
  - Single-pane toggle works
  - Tab key moves focus between panes
  - Renders correctly on macOS, Windows, Linux

### T-007: Directory Listing with TanStack Table + Virtual
- **Dependencies:** T-006
- **PRD Sections:** 13.1, 13.2, 13.9, 23.2
- **Load Agents:** desktop-ui, rust-core-architect
- **Risk Class:** R2
- **Work:**
  - Rust backend: directory listing IPC command returning FileEntry array
  - Frontend: TanStack Table rendering file list with columns (name, size, date, type)
  - TanStack Virtual for virtualized rendering
  - Sortable columns (click header to sort)
  - Resizable columns
  - View modes: list, detail, grid, compact
- **Done When:**
  - Directory with 10,000 files renders in under 500ms
  - Scrolling is smooth 60fps
  - All 4 view modes work
  - Columns are sortable and resizable
  - Memory usage under 100MB for 10K file directory

### T-008: Tabbed Browsing
- **Dependencies:** T-006
- **PRD Sections:** 13.1, 13.9
- **Load Agents:** desktop-ui
- **Risk Class:** R1
- **Work:**
  - Multiple tabs per pane
  - New tab (Cmd+T/Ctrl+T), close tab, pin tab
  - Drag to reorder tabs
  - Drag tab to other pane
  - Tab shows folder name with full path tooltip
  - Restore pinned tabs on restart
- **Done When:**
  - Tabs open, close, reorder, pin correctly
  - Pinned tabs persist across restarts
  - Middle-click closes tab
  - Tab state is independent per pane

### T-009: Navigation (Tree, Breadcrumb, Favorites, Sidebar)
- **Dependencies:** T-007
- **PRD Sections:** 13.1, 13.9
- **Load Agents:** desktop-ui, rust-core-architect
- **Risk Class:** R1
- **Work:**
  - Sidebar with: Favorites, Devices (drives), Recent locations
  - Tree view panel with lazy-loading hierarchy
  - Breadcrumb bar with clickable path segments
  - Drag-and-drop folders to Favorites
  - Mounted drives (internal, external, removable) as sidebar entries
  - Bookmarks with named groups
- **Done When:**
  - Sidebar shows favorites, devices, recents
  - Tree view expands/collapses with lazy loading
  - Breadcrumb navigates on click
  - Drives appear with name, free space, eject button
  - Favorites persist across restarts

### T-010: Core File Operations
- **Dependencies:** T-007
- **PRD Sections:** 13.3, 13.9
- **Load Agents:** desktop-ui, rust-core-architect
- **Risk Class:** R4
- **Work:**
  - Rust backend: copy, move, rename, duplicate, delete, create folder, create file
  - Progress reporting for operations longer than 1 second
  - Undo support (10 levels) for rename, move, copy, delete
  - Delete sends to OS trash by default; Shift+Delete for permanent with confirmation
  - All operations support multi-selection
  - Operations cancellable mid-progress
- **Done When:**
  - All 7 operations work correctly
  - Undo reverses last 10 operations
  - Delete uses OS trash
  - Progress indicator shows for large operations
  - Operations work across panes
  - Cross-platform tested (macOS, Windows, Linux)

### T-011: Multi-Select and Drag-and-Drop
- **Dependencies:** T-010
- **PRD Sections:** 13.3, 13.9
- **Load Agents:** desktop-ui
- **Risk Class:** R2
- **Work:**
  - Click, Shift+Click range, Cmd/Ctrl+Click toggle, Select All, Invert, Select by Pattern
  - Drag between panes (copy cross-volume, move same-volume)
  - Drag to tab header navigates and drops
  - Drag from/to desktop and native file managers
  - Visual feedback showing drop target and operation type
  - React Aria accessible drag-and-drop
- **Done When:**
  - All selection modes work
  - Drag-and-drop works between panes, to desktop, from desktop
  - Drop target highlighting is visible
  - Screen reader announces drag operations
  - Selection count shows in status bar

### T-012: Search and Filter
- **Dependencies:** T-007
- **PRD Sections:** 13.4, 13.9
- **Load Agents:** desktop-ui, rust-core-architect
- **Risk Class:** R2
- **Work:**
  - Filter bar: real-time filter-as-you-type (under 100ms)
  - Deep recursive search: filename pattern, extension, size range, date range
  - Results appear incrementally (streaming)
  - Saved searches
  - Search results show full path with Reveal in browser action
- **Done When:**
  - Filter works in under 100ms on 10K files
  - Recursive search returns streaming results
  - Searches can be saved and re-executed
  - Search across multiple locations works

### T-013: Context Menus, Keyboard Nav, Command Palette
- **Dependencies:** T-010, T-011
- **PRD Sections:** 13.9, 21.6
- **Load Agents:** desktop-ui
- **Risk Class:** R1
- **Work:**
  - Right-click: Open, Open With, Preview, Copy, Move, Rename, Delete, Compress, Copy Path, Properties
  - Context menus are mode-aware (Simple shows fewer items)
  - Arrow keys navigate, Enter opens, Backspace goes up
  - All operations have keyboard shortcuts (customizable)
  - Command Palette (Cmd+K/Ctrl+K) with fuzzy search
- **Done When:**
  - Context menus show correct items per mode
  - Full keyboard navigation works without mouse
  - Command palette finds any action/setting/path
  - Shortcuts are customizable in Settings

### T-014: Dark/Light Theme and Accessibility
- **Dependencies:** T-005
- **PRD Sections:** 13.9, 21.6, 23.2
- **Load Agents:** desktop-ui
- **Risk Class:** R1
- **Work:**
  - Light, Dark, System (auto-follows OS) themes
  - High-contrast mode (WCAG 2.1 AA)
  - All text meets 4.5:1 contrast ratio
  - Reduced motion respects OS preference
  - Screen reader labels on all interactive elements
  - Focus indicators visible on all controls
- **Done When:**
  - Theme switching is instant
  - High contrast mode passes WCAG 2.1 AA
  - VoiceOver (macOS), NVDA (Windows), Orca (Linux) can navigate file list
  - Reduced motion disables all animations when OS preference set

---

## Phase 3: Transfer Engine Core

### T-015: Transfer Queue Engine
- **Dependencies:** T-003, T-010
- **PRD Sections:** 14.4, 14.5
- **Load Agents:** rust-core-architect, desktop-ui
- **Risk Class:** R3
- **Work:**
  - Rust: async transfer queue with per-item state tracking
  - States: queued, active, paused, completed, failed, cancelled
  - Pause/resume individual items or entire queue
  - Cancel individual or all
  - Priority ordering (drag to reorder, set high/normal/low)
  - Bandwidth throttling (global and per-connection)
  - Persistent queue state across restart (crash-safe)
  - Frontend: transfer panel (mini mode: progress bar; full mode: per-item detail)
- **Done When:**
  - Queue persists across crash/restart
  - Pause/resume works on individual and batch
  - Bandwidth throttling limits transfer speed as configured
  - Transfer panel shows progress, ETA, throughput
  - 100+ concurrent items in queue without UI freeze

### T-016: Resume and Retry Logic
- **Dependencies:** T-015
- **PRD Sections:** 14.4
- **Load Agents:** rust-core-architect
- **Risk Class:** R3
- **Work:**
  - Auto-resume interrupted transfers from last byte offset
  - Checksum of completed portion before continuing
  - Retry policy: configurable count (default 3) with exponential backoff
  - Auto-reconnect dropped connections
  - Per-item failure does not stop entire queue
  - Failed items collected with Retry All Failed button
- **Done When:**
  - Transfer resumes from correct byte offset after disconnect
  - Retry with backoff works (1s, 5s, 30s)
  - Single file failure does not block other transfers
  - Retry All Failed re-queues all failed items

### T-017: Conflict Resolution
- **Dependencies:** T-015
- **PRD Sections:** 14.4, 15.4
- **Load Agents:** rust-core-architect, desktop-ui
- **Risk Class:** R4
- **Work:**
  - Policies: overwrite always, skip always, overwrite if newer, overwrite if larger, rename (append number), ask each time
  - Policy settable per-transfer, per-connection, or global default
  - Ask mode shows side-by-side comparison (name, size, date)
  - Apply to all checkbox in ask mode
  - Conflict resolution logged in transfer summary
- **Done When:**
  - All 6 policies work correctly
  - Ask mode shows clear comparison
  - Apply-to-all batches remaining conflicts
  - Conflict decisions appear in transfer history

### T-018: Post-Transfer Verification and History
- **Dependencies:** T-015
- **PRD Sections:** 14.4
- **Load Agents:** rust-core-architect, desktop-ui
- **Risk Class:** R2
- **Work:**
  - Optional checksum verification (MD5, SHA-256) after transfer
  - Mismatch triggers alert + auto-retry option
  - Transfer history: timestamp, source, dest, size, duration, speed, status, errors
  - History searchable by path, date, status, connection
  - Export history as CSV/JSON
  - 90-day retention (configurable)
- **Done When:**
  - Checksum verification detects corrupted transfer
  - History stores all transfer metadata
  - Search and filter work on history
  - Export produces valid CSV/JSON

### T-019: Connection Manager
- **Dependencies:** T-003
- **PRD Sections:** 14.7
- **Load Agents:** rust-core-architect, desktop-ui, security-review
- **Risk Class:** R3
- **Work:**
  - Save connections: name, type, host/endpoint, credentials, per-connection settings
  - Connection groups (named folders)
  - One-click connection test
  - Bookmarks per connection (frequently used paths)
  - Quick-connect bar with autocomplete
  - Import/export connections (encrypted JSON)
  - Credentials stored in OS keychain (macOS Keychain, Windows Credential Vault, Linux Secret Service)
  - Encrypted vault fallback when keychain unavailable
- **Done When:**
  - Connections save, load, test, group correctly
  - Credentials use OS keychain on all 3 platforms
  - Fallback vault works when keychain unavailable
  - Import/export produces encrypted file
  - OAuth tokens stored securely and auto-refresh

---

## Phase 4: Protocol Connectors (Can Parallelize)

### T-020: SFTP Connector
- **Dependencies:** T-015, T-019
- **PRD Sections:** 14.1, 14.2
- **Load Agents:** connector-protocol, test-and-qa, security-review
- **Risk Class:** R3
- **Work:**
  - Connection: host, port, username, auth (password/SSH key/SSH agent)
  - SSH key support: RSA, Ed25519, ECDSA, passphrase-protected
  - Jump host (ProxyJump) support
  - Host key verification with known_hosts database
  - File operations: browse, upload, download, rename, delete, mkdir
  - Resume using SFTP extensions where supported
  - Keepalive packets for idle connections
  - Connection health indicator (latency, status)
- **Done When:**
  - All auth methods work
  - Jump host works
  - Host key verification prompts on first connect
  - Resume works after disconnect
  - All file operations work transparently
  - Negative tests: wrong password, expired key, unreachable host

### T-021: FTP/FTPS Connector
- **Dependencies:** T-015, T-019
- **PRD Sections:** 14.1, 14.2
- **Load Agents:** connector-protocol, test-and-qa, security-review
- **Risk Class:** R3
- **Work:**
  - Active and passive mode
  - TLS: none, explicit, implicit
  - Directory listing cache
  - Resume using REST command
  - Certificate validation with self-signed cert option
- **Done When:**
  - Active and passive mode work
  - FTPS explicit and implicit TLS work
  - Resume works after interruption
  - Self-signed cert acceptance works with user confirmation

### T-022: WebDAV Connector
- **Dependencies:** T-015, T-019
- **PRD Sections:** 14.1, 14.2
- **Load Agents:** connector-protocol, test-and-qa
- **Risk Class:** R3
- **Work:**
  - Auth: basic, digest, OAuth
  - HTTPS enforced by default (HTTP requires opt-in with warning)
  - WebDAV locking where server supports it
  - Chunked upload for large files
  - Compatible with Nextcloud, ownCloud
- **Done When:**
  - All auth methods work
  - HTTPS enforcement works
  - Large file upload uses chunked encoding
  - Nextcloud/ownCloud tested

### T-023: SMB/CIFS Connector
- **Dependencies:** T-015, T-019
- **PRD Sections:** 14.1, 14.2, 14.8
- **Load Agents:** connector-protocol, test-and-qa, security-review
- **Risk Class:** R3
- **Work:**
  - Connection: hostname/IP, share name, username, password, domain
  - Network discovery of SMB shares on LAN
  - Kerberos auth for domain environments
  - SMB2/SMB3 preferred, SMB1 disabled by default
  - NAS devices in discovery sidebar
- **Done When:**
  - Connect and browse SMB shares
  - Discovery finds shares on LAN (where OS permits)
  - SMB3 preferred, SMB1 requires explicit opt-in
  - Works on macOS, Windows, Linux

### T-024: NFS Connector
- **Dependencies:** T-015, T-019
- **PRD Sections:** 14.1, 14.2
- **Load Agents:** connector-protocol, test-and-qa
- **Risk Class:** R3
- **Work:**
  - Connection: hostname, export path, NFS version (v3/v4)
  - Unix permission display (rwxrwxrwx)
  - Appears as mount point in sidebar under Network
- **Done When:**
  - NFSv3 and NFSv4 connections work
  - File operations work transparently
  - Permissions display correctly

### T-025: Drive-to-Drive and Local Transfer
- **Dependencies:** T-015, T-017
- **PRD Sections:** 14.3, 14.5
- **Load Agents:** rust-core-architect, desktop-ui
- **Risk Class:** R2
- **Work:**
  - Same transfer queue framework for local drive transfers
  - Preflight: space check, filesystem compatibility, duplicate risk, permissions
  - All local transfer contexts: internal-to-internal, internal-to-external, external-to-external, removable-to-removable
  - Sidebar devices section: name, icon, total/free space, filesystem type, eject
- **Done When:**
  - Drive-to-drive transfer uses same queue as remote
  - Preflight catches space/permission issues
  - All drive combinations work
  - Sidebar shows all connected drives with metadata

---

## Phase 5: Cloud Connectors (Can Parallelize)

### T-026: Amazon S3 Connector
- **Dependencies:** T-015, T-019
- **PRD Sections:** 14.1, 14.2
- **Load Agents:** connector-protocol, test-and-qa, security-review
- **Risk Class:** R3
- **Work:**
  - Auth: access key, secret key, region, IAM role
  - Bucket listing as top-level folders
  - Multipart upload for files over 5MB
  - Resume via range requests
  - Presigned URL generation
  - S3-compatible endpoints (MinIO, Wasabi, Backblaze B2 S3 mode)
- **Done When:**
  - Connect, browse, upload/download work
  - Multipart upload works for large files
  - S3-compatible endpoints (MinIO, Wasabi) work
  - Presigned URL generation works

### T-027: Google Drive Connector
- **Dependencies:** T-015, T-019
- **PRD Sections:** 14.1, 14.2
- **Load Agents:** connector-protocol, test-and-qa, security-review
- **Risk Class:** R3
- **Work:**
  - OAuth 2.0 flow (opens browser)
  - My Drive, Shared Drives, Shared with Me
  - Google Docs/Sheets export options (PDF, DOCX, etc.)
  - File versioning: view history, restore
  - Auto token refresh
- **Done When:**
  - OAuth flow completes successfully
  - All drive types browsable
  - Upload/download work
  - Token refresh works without re-auth

### T-028: Dropbox Connector
- **Dependencies:** T-015, T-019
- **PRD Sections:** 14.1, 14.2
- **Load Agents:** connector-protocol, test-and-qa, security-review
- **Risk Class:** R3
- **Work:**
  - OAuth 2.0 flow
  - Root, team folders, shared folders browsable
  - File operations: upload, download, rename, move, delete, mkdir
  - Selective sync support
- **Done When:**
  - OAuth flow works
  - All folder types browsable
  - All file operations work
  - Drag-and-drop between Dropbox and local panes

### T-029: OneDrive Connector
- **Dependencies:** T-015, T-019
- **PRD Sections:** 14.1, 14.2
- **Load Agents:** connector-protocol, compatibility-engine, test-and-qa, security-review
- **Risk Class:** R3
- **Work:**
  - OAuth 2.0 (personal) and Azure AD (business)
  - SharePoint document libraries
  - OneDrive naming restriction profile auto-applied (400-char path, forbidden chars)
  - Compatibility warnings surface immediately on drag
- **Done When:**
  - Personal and business auth work
  - SharePoint libraries browsable
  - Naming restrictions detected in preflight
  - Upload/download work with large files

### T-030: Backblaze B2 Connector
- **Dependencies:** T-015, T-019
- **PRD Sections:** 14.1, 14.2
- **Load Agents:** connector-protocol, test-and-qa
- **Risk Class:** R3
- **Work:**
  - Auth: application key ID + application key
  - Bucket listing and virtual folder browsing
  - Large file API for files over 100MB
  - B2 S3-compatible mode as alternative path
- **Done When:**
  - Connect, browse, upload/download work
  - Large file API works for large files

---

## Phase 6: Peer Transfer and Migration

### T-031: Peer Discovery and Computer-to-Computer Transfer
- **Dependencies:** T-015, T-019, T-025
- **PRD Sections:** 14.3, 14.8, 22.6
- **Load Agents:** connector-protocol, desktop-ui, rust-core-architect
- **Risk Class:** R3
- **Work:**
  - mDNS/Bonjour/Avahi peer discovery on LAN
  - Discovered devices: hostname, OS, IP, app version, online/offline
  - Encrypted transfer channel (TLS)
  - Accept/deny prompt on receiving device with Trust this device option
  - Saved peers in sidebar
  - Manual IP:port fallback when discovery unavailable
  - Guided Transfer to another computer flow
- **Done When:**
  - Peer appears within 30 seconds on same LAN
  - Transfer works between macOS-Windows, macOS-Linux, Windows-Linux
  - Encrypted channel established
  - Manual fallback works when discovery fails
  - Guided flow completes end-to-end

### T-032: Migration Workflows
- **Dependencies:** T-031, T-025, T-017
- **PRD Sections:** 14.9, 22.7, 22.8
- **Load Agents:** migration-workflow, desktop-ui, compatibility-engine
- **Risk Class:** R4
- **Work:**
  - Computer migration wizard: connect source, select folders, space analysis, compatibility preflight, preview, execute, summary
  - Drive migration wizard: select source drive, select dest drive, filesystem compat check, duplicate detection, execute, verify
  - External backup wizard: select source, select external drive, choose mode (copy/mirror/sync/versioned), execute, verify
  - All migrations: resumable, retry failed items, post-run summary
- **Done When:**
  - Computer migration works across mixed OS
  - Drive migration detects filesystem differences
  - Backup wizard offers all 4 modes
  - Resume works after interruption
  - Summary accurately reports results

### T-033: Server-to-Server Direct Transfer
- **Dependencies:** T-020, T-026
- **PRD Sections:** 14.6
- **Load Agents:** connector-protocol, rust-core-architect
- **Risk Class:** R2
- **Work:**
  - Direct transfer between remote endpoints where possible (SFTP to SFTP, S3 to S3)
  - Fallback to local relay when direct impossible
  - Clear indication of transfer method in queue
- **Done When:**
  - Direct transfer works for supported protocol pairs
  - Fallback relay works transparently
  - User sees whether transfer is direct or relayed

---

## Phase 7: Naming Compatibility Engine

### T-034: Unicode Normalizer and Character Sanitizer
- **Dependencies:** T-002
- **PRD Sections:** 16.1, 16.2, 16.7, 16.8
- **Load Agents:** compatibility-engine, test-and-qa
- **Risk Class:** R4
- **Work:**
  - Unicode normalizer: NFC to NFD conversion based on source/destination OS
  - Handle: Hangul jamo, combining diacriticals, zero-width joiners
  - Character sanitizer: Windows-forbidden chars to full-width Unicode equivalents
  - Reversible mapping stored in database
  - Trailing dots and spaces detection
  - Test corpus: 100+ filenames with Latin accents, CJK, emoji, Arabic, Hebrew, mixed scripts
- **Done When:**
  - 100+ filename test corpus passes
  - NFC to NFD round-trip preserves accessibility via mapping
  - Character replacement is reversible
  - Performance: normalize 10,000 filenames in under 100ms

### T-035: Reserved Names, Path Length, Case Handling
- **Dependencies:** T-034
- **PRD Sections:** 16.2, 16.7
- **Load Agents:** compatibility-engine, test-and-qa
- **Risk Class:** R4
- **Work:**
  - Windows reserved names: CON, PRN, AUX, NUL, COM1-9, LPT1-9 (case-insensitive, with/without extension)
  - Escape with underscore prefix: CON.txt becomes _CON.txt
  - Path length: Windows MAX_PATH (260), extended (32,767), ext4 (255 bytes per component), OneDrive (400 decoded)
  - Path shortening: truncate preserving extension + 4-char hash suffix
  - Case collision: Report.txt vs report.txt detected on case-insensitive targets
  - Filesystem type detection via OS APIs
- **Done When:**
  - All Windows reserved names detected and escaped
  - Path length validation works per target profile
  - Case collisions detected between source files
  - CJK 3-byte UTF-8 awareness works for ext4 255-byte limit

### T-036: Destination Profiles and Cloud Rules
- **Dependencies:** T-034, T-035
- **PRD Sections:** 16.5, 16.7
- **Load Agents:** compatibility-engine, connector-protocol
- **Risk Class:** R3
- **Work:**
  - Create 10+ profiles: Windows-NTFS, Windows-FAT32, macOS-APFS-CI, APFS-CS, ext4, OneDrive, Google Drive, Dropbox, S3, Backblaze B2, SMB-to-Windows, strict-enterprise
  - Each profile: forbidden chars, reserved names, max path, max filename, case sensitivity, Unicode pref, trailing char rules
  - Auto-detection of destination profile from filesystem/connector type
  - Profile chaining (e.g., SMB-to-Windows applies both transport + destination rules)
  - Profiles versioned and updateable via app update
- **Done When:**
  - All profiles defined and tested
  - Auto-detection works for all connector types
  - Profile chaining works
  - Cloud provider profiles match current provider restrictions

### T-037: Mapping Database and Name Restoration
- **Dependencies:** T-034, T-035, T-003
- **PRD Sections:** 16.3, 16.6
- **Load Agents:** compatibility-engine, rust-core-architect
- **Risk Class:** R4
- **Work:**
  - SQLite mapping table: original_name, translated_name, source_path, dest_path, rules_applied, timestamp, reversible flag
  - Mapping created for every Tier 1 and Tier 2 translation
  - Restore operation: offer original names when files return to compatible destination
  - Optional sidecar manifest (.ufop-manifest.json)
  - Optional xattr storage on macOS/Linux
  - Mapping retention: configurable, default 1 year
- **Done When:**
  - Mapping records persist for all translations
  - Restore operation works end-to-end
  - Sidecar manifest writes and reads correctly
  - Search works on both logical and physical names

### T-038: Intervention Tiers and Compatibility UX
- **Dependencies:** T-036, T-037
- **PRD Sections:** 16.4
- **Load Agents:** compatibility-engine, desktop-ui
- **Risk Class:** R4
- **Work:**
  - Tier 1 (auto-quiet): handle silently, log, badge count in summary
  - Tier 2 (visible auto): inline notification during transfer, non-blocking, expandable details
  - Tier 3 (decision required): modal dialog with options (rename, skip, overwrite with warning, cancel)
  - Simple mode messages: We adjusted 8 names so the transfer could continue
  - Advanced mode: full detail view with original to translated + reason
  - Apply to all similar for batch Tier 3 resolution
  - Badge/icon on files with active translations
- **Done When:**
  - All 3 tiers render correctly
  - Simple mode shows plain-language summary
  - Advanced mode shows full technical detail
  - Tier 3 pauses only affected items, not entire queue
  - All decisions logged in audit trail

---

## Phase 8: Sync Engine

### T-039: Sync Pair Creation and Core Modes
- **Dependencies:** T-015, T-003
- **PRD Sections:** 15.1, 15.2, 15.6
- **Load Agents:** sync-engine, desktop-ui
- **Risk Class:** R3
- **Work:**
  - Sync pair wizard (Simple mode) and direct setup (Advanced)
  - Modes: one-way, two-way, mirror, versioned backup
  - Execution: manual, scheduled (cron-like), watch-based (filesystem watcher), API/CLI-triggered
  - Selective sync: include/exclude patterns (glob, regex), size filter, type filter
  - All sync contexts: local to local, local to external, local to remote, local to cloud, local to peer
- **Done When:**
  - All 4 sync modes work correctly
  - Watch-based triggers within 5 seconds of file change
  - Scheduled sync runs at configured times
  - Selective sync correctly includes/excludes
  - Sync pair appears in sidebar with status indicator

### T-040: Sync Dry-Run and Preview
- **Dependencies:** T-039
- **PRD Sections:** 15.3
- **Load Agents:** sync-engine, desktop-ui
- **Risk Class:** R2
- **Work:**
  - Dry-run preview: files to add, modify, delete, skip
  - Categories: new, changed, deletions, conflicts, compatibility translations, collision risks, path-length warnings
  - Each category: count + expandable file list
  - Conflict items: source vs destination with size/date diff
  - Compatibility items: original to adjusted with reason
  - Approval step: Proceed / Cancel / Modify settings
  - Export preview as CSV/PDF
- **Done When:**
  - Dry-run completes in under 30 seconds for under 10K file pairs
  - Preview accurately reflects what sync will do
  - Export produces valid report
  - Approval gates actual execution

### T-041: Sync Conflict Resolution
- **Dependencies:** T-039, T-017
- **PRD Sections:** 15.4
- **Load Agents:** sync-engine, desktop-ui
- **Risk Class:** R4
- **Work:**
  - Policies: ask, source wins, dest wins, newest wins, create conflict copy, skip, quarantine for approval
  - Simple mode default: create conflict copy with (conflict YYYY-MM-DD) suffix
  - Enterprise: policy-driven default
  - Per-sync-pair policy override
  - Ask mode: side-by-side comparison
  - Quarantine mode: review queue in Sync panel
- **Done When:**
  - All 7 policies work deterministically
  - Simple mode creates conflict copies by default
  - Quarantine queue is accessible and actionable
  - Conflict resolution logged in sync history

### T-042: Sync Verification, Rollback, and Reporting
- **Dependencies:** T-039, T-041
- **PRD Sections:** 15.5, 15.7
- **Load Agents:** sync-engine, rust-core-architect
- **Risk Class:** R4
- **Work:**
  - Checksum verification toggle per pair
  - Fast compare (size + timestamp) and full compare (byte/checksum)
  - Pre-destructive snapshot: store metadata before delete/overwrite
  - Rollback: undo last sync run (restore deleted, revert overwrites) within 7 days
  - Partial failure continuation: one file failure does not stop sync
  - Resumable sync state after interruption
  - Per-run reporting: start/end time, duration, files added/modified/deleted, conflicts, errors, bytes transferred
  - Sync health indicator: green/yellow/red/gray in sidebar
- **Done When:**
  - Checksum verification catches corruption
  - Rollback restores deleted files within retention period
  - Partial failure continues and collects failed items
  - Resume works after app crash mid-sync
  - Reporting matches actual actions taken

---

## Phase 9: AI, Terminal, Encryption

### T-043: AI Assistant - Error Explanations and Suggestions
- **Dependencies:** T-015, T-039
- **PRD Sections:** 20.1, 20.2
- **Load Agents:** desktop-ui, rust-core-architect
- **Risk Class:** R2
- **Work:**
  - AI chat panel in sidebar
  - Right-click failed item then Explain this error
  - Plain-language error explanations (under 3 second response)
  - AI suggestions: sync rules, exclusions, cleanup, folder classification, duplicate risk
  - Suggestion cards with Accept/Dismiss
  - Local context only (error logs, transfer history) unless user enables broader analysis
  - Fallback when AI unavailable: structured error details without AI formatting
- **Done When:**
  - AI explains transfer/sync errors in plain language
  - Suggestions are actionable and dismissible
  - Response time under 3 seconds
  - Fallback works when AI service unavailable
  - No file content sent to AI without explicit opt-in

### T-044: AI Automation - Natural Language Job Creation
- **Dependencies:** T-043
- **PRD Sections:** 20.2
- **Load Agents:** desktop-ui
- **Risk Class:** R2
- **Work:**
  - Natural language input: Sync my Documents to Google Drive every night
  - AI parses and generates sync pair config
  - Preview for user approval before activating
  - Supports: sync pairs, transfers, backups, exclusions, rename patterns
  - Ambiguity handling: AI asks clarifying questions
- **Done When:**
  - NL to sync/transfer config works for common intents
  - User sees preview before anything activates
  - Generated configs are editable in Advanced mode
  - AI asks clarifying questions for ambiguous input

### T-045: AI Safety and Privacy Controls
- **Dependencies:** T-043
- **PRD Sections:** 20.3, 20.4
- **Load Agents:** security-review, desktop-ui
- **Risk Class:** R3
- **Work:**
  - Destructive actions require confirmation button press
  - AI-generated rules previewable before activation
  - All AI actions logged in audit trail
  - Enterprise: AI master toggle, per-feature toggles, content analysis opt-in, model routing config
  - AI governance settings in admin console
- **Done When:**
  - No destructive AI action executes without user confirmation
  - All AI interactions are auditable
  - Enterprise toggles work per-org
  - Content analysis is off by default, opt-in only

### T-046: Built-In Terminal
- **Dependencies:** T-005, T-019
- **PRD Sections:** 11.1
- **Load Agents:** terminal-shell-integration, desktop-ui, security-review
- **Risk Class:** R3
- **Work:**
  - xterm.js frontend + Rust PTY backend
  - Local terminal: system shell (bash/zsh on macOS/Linux, PowerShell on Windows)
  - Remote terminal: SSH to any connected SFTP/SSH server
  - Split terminal (horizontal/vertical)
  - Drag file from browser pastes escaped path
  - Terminal sessions persist across tab switches
- **Done When:**
  - Terminal launches on all 3 platforms with correct shell
  - Remote SSH session works using saved SFTP credentials
  - Split pane works
  - Drag-to-terminal pastes path
  - Terminal isolated from file-ops UI

### T-047: Encryption (In-Transit and At-Rest)
- **Dependencies:** T-019, T-003
- **PRD Sections:** 18.3
- **Load Agents:** rust-core-architect, security-review
- **Risk Class:** R4
- **Work:**
  - In-transit: TLS 1.3 enforcement, FTPS auto-upgrade, SMB3 preferred, all cloud APIs HTTPS
  - At-rest: AES-256-GCM encrypted vaults (appear as regular folders)
  - Encrypt-before-upload (zero-knowledge cloud mode)
  - Key management: master password + Argon2id KDF, optional YubiKey/FIDO2
  - Cryptomator vault compatibility (read/write)
  - Selective encryption: specific folders/patterns
  - Enterprise: encryption policies per connector/destination
- **Done When:**
  - All protocols use encrypted transport
  - Encrypted vaults create, read, write correctly
  - Cryptomator vaults open natively
  - Key derivation uses Argon2id
  - Enterprise encryption policies enforceable
  - Security review signoff obtained

---

## Phase 10: Enterprise and Governance

### T-048: Admin Console Shell
- **Dependencies:** T-001 (separate Next.js app)
- **PRD Sections:** 17, 21.1
- **Load Agents:** admin-console, ci-devops
- **Risk Class:** R2
- **Work:**
  - Next.js full app with auth
  - Navigation: Dashboard, Users/Roles, Devices, Policies, Approvals, Audit, Connectors, AI Governance, Billing
  - Responsive design (desktop + tablet browsers)
  - Shared design tokens from @ufop/design-tokens
- **Done When:**
  - Admin console accessible via browser with auth
  - All navigation sections render
  - Shared design tokens match desktop app appearance
  - API-first: all actions available via REST

### T-049: RBAC and User Management
- **Dependencies:** T-048
- **PRD Sections:** 17.1
- **Load Agents:** admin-console, security-review
- **Risk Class:** R3
- **Work:**
  - Roles: org owner, org admin, security admin, approver, operator, user, read-only auditor
  - User management: invite, assign role, deactivate, remove
  - SSO: SAML 2.0 (Business), full SAML + SCIM (Enterprise)
  - Bulk user management via CSV
- **Done When:**
  - All 7 roles have defined, enforced permissions
  - SSO/SAML login works
  - Role restrictions enforced in both admin console and desktop app
  - Bulk CSV import/export works

### T-050: Policy Engine
- **Dependencies:** T-049
- **PRD Sections:** 17.2
- **Load Agents:** admin-console, rust-core-architect, security-review
- **Risk Class:** R3
- **Work:**
  - Policy domains: allowed connectors, approved destinations, encryption requirements, max transfer size, file-type restrictions, checksum requirements, naming strictness, sync conflict policies, deletion restrictions, AI usage, retention
  - Policies assignable to: entire org, specific roles, specific users, connection types
  - Policy precedence: user then role then org
  - Policies propagate to desktop clients within 5 minutes
  - Desktop shows active policies in Settings then Organization (read-only)
- **Done When:**
  - All 11 policy domains configurable
  - Policies propagate to desktop clients
  - Desktop app enforces policies
  - Policy changes logged in audit trail

### T-051: Approval Workflows
- **Dependencies:** T-050
- **PRD Sections:** 17.3, 22.3
- **Load Agents:** admin-console, desktop-ui
- **Risk Class:** R3
- **Work:**
  - Triggers: external destination transfer, destructive sync, policy exception, Tier 3 naming, sensitive file movement
  - Request includes: operation, source, dest, file count/size, reason, triggering policy
  - Approver notification (in-app + email)
  - States: pending, approved, denied, expired (48hr default)
  - On approval: auto-execute or notify user
  - Full audit trail: requester, approver, decision, timestamp
- **Done When:**
  - Approval flow works end-to-end
  - Approver sees pending requests in admin console
  - User sees request status in desktop app
  - Expired requests auto-deny
  - Audit trail complete

### T-052: Audit Logging and Device Health
- **Dependencies:** T-048, T-003
- **PRD Sections:** 17.4, 17.5
- **Load Agents:** admin-console, rust-core-architect, security-review
- **Risk Class:** R3
- **Work:**
  - Audit events: login/logout, connection CRUD, transfers, syncs, naming translations, approvals, policy changes, admin actions, API/CLI actions
  - Audit explorer: searchable, filterable by date/user/action/status
  - Export: CSV, JSON, SIEM webhook/syslog (Enterprise)
  - Immutable: users cannot edit/delete their own entries
  - Device health: client version, last seen, last sync, failure patterns, policy compliance, connector health
  - Fleet view: total devices, online count, out-of-date count, non-compliant count
- **Done When:**
  - All event types are captured
  - Audit explorer search and filter work
  - Export produces valid data
  - Device health dashboard shows real device data
  - Audit entries are immutable

### T-053: Shared Workspaces
- **Dependencies:** T-049, T-050
- **PRD Sections:** 17.6
- **Load Agents:** admin-console
- **Risk Class:** R2
- **Work:**
  - Shared connections appear automatically in team member desktop apps
  - Shared sync templates activatable by team members
  - Shared automation templates
  - Changes propagate within 5 minutes
- **Done When:**
  - Shared connections appear in desktop sidebar under Organization
  - Templates are activatable
  - Propagation works within 5 minutes

---

## Phase 11: UX Polish and Simple Mode

### T-054: Simple Mode Implementation
- **Dependencies:** T-006 through T-013 (file manager core)
- **PRD Sections:** 9.1, 21.1, 21.6
- **Load Agents:** desktop-ui, product-consistency
- **Risk Class:** R2
- **Work:**
  - Simple mode is default for new installs
  - Navigation: Files, Transfers, Sync, Cloud and Servers, Search, Favorites, Activity, Settings
  - Advanced features hidden: regex rename, checksums, detailed connections, automation, naming inspector, scripting, logs
  - Mode switch: Settings then User Mode (instant, no restart)
  - Larger touch targets, more whitespace, fewer controls
- **Done When:**
  - Simple mode renders clean interface
  - Advanced features are hidden but discoverable via More options
  - Mode switch is instant
  - General user can browse, transfer, sync without Advanced mode

### T-055: Onboarding Wizard
- **Dependencies:** T-054
- **PRD Sections:** 21.2
- **Load Agents:** desktop-ui, documentation
- **Risk Class:** R1
- **Work:**
  - First-run wizard: Welcome, Choose style (Personal/Power/Work), Connect locations, First action, Explain compat, Enter workspace
  - Skip option at every step
  - Re-accessible from Help then Getting Started
- **Done When:**
  - Wizard launches on first open
  - All 6 steps work
  - Skip works at every step
  - OAuth cloud connection works from wizard

### T-056: All 15 Guided Flows
- **Dependencies:** T-054
- **PRD Sections:** 21.3
- **Load Agents:** desktop-ui, documentation
- **Risk Class:** R1
- **Work:**
  - 15 wizards: connect OneDrive/GDrive/Dropbox, connect local sync pair, connect SMB, connect SFTP, start transfer, create sync, resolve interruption, handle compat warning, restore names, export report, transfer to computer, copy to external, old to new drive, backup folder, migrate computers
  - Each: step indicator, back/next/cancel, contextual help, validation per step
  - Accessible from: main menu, context menu, sidebar, command palette, AI assistant
- **Done When:**
  - All 15 wizards functional in Simple mode
  - Validation prevents invalid input progression
  - Wizards accessible from all listed entry points

### T-057: Content Design and Plain Language
- **Dependencies:** T-054
- **PRD Sections:** 21.4, 21.5
- **Load Agents:** desktop-ui, documentation, product-consistency
- **Risk Class:** R1
- **Work:**
  - Review all user-facing strings against content design rules
  - Every error: what happened, why, what app did, what user can do
  - No unexplained acronyms in Simple mode
  - Destructive operations show clear consequences
  - Recovery guidance includes concrete action (button/link/instruction)
  - Undo everywhere: rename, move, copy, delete (from trash)
  - Recent activity feed (last 100 operations)
- **Done When:**
  - Content review pass complete
  - All errors follow the what/why/did/next pattern
  - Undo works for all listed operations
  - Activity feed shows recent history

---

## Phase 12: CLI, API, and Final Integration

### T-058: CLI Implementation
- **Dependencies:** T-015, T-039
- **PRD Sections:** 19.1
- **Load Agents:** api-cli, test-and-qa
- **Risk Class:** R2
- **Work:**
  - Binary: ufop (standalone Rust, installable via package managers)
  - Commands: login, connection list/add/test, transfer src dest, sync list/run/create, compat check
  - Flags: --dry-run, --json, --yaml, --limit rate
  - Exit codes: 0 success, 1 partial, 2 full failure, 3 auth error, 4 policy violation
  - Progress bar in human mode, JSON events in --json mode
- **Done When:**
  - All commands work as documented
  - Structured output is parseable
  - Exit codes are correct
  - Progress reporting works in both modes

### T-059: REST API
- **Dependencies:** T-048, T-052
- **PRD Sections:** 19.2
- **Load Agents:** api-cli, admin-console, security-review
- **Risk Class:** R3
- **Work:**
  - Auth: API keys (Business), OAuth 2.0 client credentials (Enterprise)
  - Endpoints: users, roles, devices, policies, approvals, audit, connectors, AI governance
  - Transfer/sync trigger endpoints
  - Webhook support for events
  - Rate limiting (1000 req/min default)
  - OpenAPI 3.0 spec auto-generated
- **Done When:**
  - All admin endpoints work
  - Transfer/sync triggers work via API
  - Webhooks fire for configured events
  - OpenAPI spec is accurate and published

### T-060: Batch Rename Engine
- **Dependencies:** T-010
- **PRD Sections:** 13.5
- **Load Agents:** desktop-ui, rust-core-architect
- **Risk Class:** R2
- **Work:**
  - Live preview of all changes before applying
  - Tokens: name, ext, num, date, parent, counter
  - Find/Replace with regex (Advanced) and literal (Simple)
  - Sequential numbering with start/step/zero-padding
  - Case transformation: UPPER, lower, Title, Sentence
  - Date insertion from file metadata
  - Undo renames back to originals
  - Preview uses compatibility engine to warn about cross-platform issues
- **Done When:**
  - Live preview updates as user types
  - All token types work
  - Regex works in Advanced mode
  - Undo restores original names
  - Compatibility warnings show for problematic names

### T-061: Preview Engine
- **Dependencies:** T-007
- **PRD Sections:** 13.6
- **Load Agents:** desktop-ui, security-review
- **Risk Class:** R3
- **Work:**
  - Preview pane toggle or Spacebar press
  - Types: images (JPEG/PNG/GIF/WebP/SVG), PDFs, text/code (30+ languages syntax highlighting), Markdown (rendered), audio (waveform + playback), video (thumbnail + playback), archive contents, file metadata (EXIF, dimensions)
  - Sandboxed: no script execution, no network access, no write permissions
  - Render in under 500ms for files under 10MB
  - Large files (over 50MB): metadata only with option to load
- **Done When:**
  - All listed file types preview correctly
  - Preview is sandboxed (security review confirms)
  - Renders in under 500ms for standard files
  - Large files show metadata without hanging

### T-062: Archive Tools
- **Dependencies:** T-007
- **PRD Sections:** 13.7
- **Load Agents:** desktop-ui, rust-core-architect
- **Risk Class:** R2
- **Work:**
  - Double-click archive opens as virtual folder
  - Extract: right-click then Extract Here / Extract To
  - Create: right-click selected then Compress As (ZIP, TAR, TAR.GZ, 7Z)
  - Password-protected ZIP/7Z support
  - Archive operations enter queue for large archives
- **Done When:**
  - Archives browse as virtual folders
  - Extract and create work for all listed formats
  - Password-protected archives prompt correctly
  - Large archives use queue

### T-063: Integrity Tools (Checksums, Duplicates, Tags)
- **Dependencies:** T-007
- **PRD Sections:** 13.8
- **Load Agents:** desktop-ui, rust-core-architect
- **Risk Class:** R2
- **Work:**
  - Checksum: MD5/SHA-1/SHA-256 via right-click
  - Verify against provided hash or sidecar file
  - Duplicate detection: filename/size fast pass then hash confirmation
  - Duplicate view: groups with keep newest/largest/by-path/delete options
  - Tags: color-coded, stored in local DB
  - Labels: custom text
  - Smart Folders: saved filter queries
- **Done When:**
  - Checksums generate and verify correctly
  - Duplicates detected accurately
  - Tags/labels persist across restarts
  - Smart folders auto-populate

---

## Phase 13: Packaging and Launch Prep

### T-064: Installers and Auto-Update
- **Dependencies:** All previous phases
- **PRD Sections:** 23.2
- **Load Agents:** ci-devops, security-review, release-validation
- **Risk Class:** R3
- **Work:**
  - macOS: .dmg installer, Homebrew cask
  - Windows: .msi (silent install), .exe installer, winget
  - Linux: .deb, .AppImage, .rpm
  - All binaries code-signed (Apple Developer ID, Windows EV cert, Linux GPG)
  - Tauri updater: differential updates, rollback on failure
  - Update channels: stable (default), beta (opt-in)
- **Done When:**
  - Installers work on all platforms
  - Code signing verified
  - Auto-update downloads and installs correctly
  - Rollback works when update fails
  - Install size under 50MB

### T-065: Localization
- **Dependencies:** T-054
- **PRD Sections:** 23.2
- **Load Agents:** desktop-ui, documentation
- **Risk Class:** R1
- **Work:**
  - 8 languages: English, Spanish, French, German, Portuguese, Japanese, Chinese (Simplified), Korean
  - Auto-detect from OS locale with manual override
  - All UI strings, errors, guided flows, help text translated
  - Date/time/number formats respect locale
- **Done When:**
  - All 8 languages render correctly
  - Locale detection works
  - No untranslated strings in supported languages

### T-066: End-to-End Journey Validation
- **Dependencies:** All functional tasks complete
- **PRD Sections:** 22 (all 8 journeys)
- **Load Agents:** test-and-qa, release-validation
- **Risk Class:** R4
- **Work:**
  - Journey 1: Local folder to OneDrive with incompatible names
  - Journey 2: macOS to Windows share two-way sync
  - Journey 3: Enterprise-governed external transfer with approval
  - Journey 4: AI-assisted folder cleanup
  - Journey 5: Drive-to-drive transfer on same computer
  - Journey 6: Computer-to-computer transfer over LAN
  - Journey 7: Computer migration workflow
  - Journey 8: External drive backup
- **Done When:**
  - All 8 journeys complete end-to-end on all 3 platforms
  - Results match PRD descriptions
  - Performance benchmarks met (under 2s start, under 500ms 10K listing, under 100MB idle)

### T-067: Security Audit and Bug Bash
- **Dependencies:** T-066
- **PRD Sections:** 18, 25
- **Load Agents:** security-review, test-and-qa, release-validation
- **Risk Class:** R4
- **Work:**
  - Credential storage review across all 3 platforms
  - Encryption implementation review
  - Preview sandbox verification
  - Updater signing verification
  - Dependency CVE scan (no critical/high vulnerabilities)
  - Full-team bug bash
  - Bug count target: under 30 open (no P0, under 5 P1)
- **Done When:**
  - Security review signoff obtained
  - No P0 bugs
  - CVE scan clean
  - Credential storage approved
  - Updater integrity verified

### T-068: Documentation Complete
- **Dependencies:** All functional tasks
- **PRD Sections:** 21.4
- **Load Agents:** documentation, release-validation
- **Risk Class:** R0
- **Work:**
  - User docs for all Simple mode flows
  - Admin guide for all admin console features
  - CLI reference with examples
  - API reference (OpenAPI spec)
  - Troubleshooting guide
  - Migration guide
  - Release notes
- **Done When:**
  - All user-facing features documented
  - CLI/API references match actual behavior
  - Release notes list all notable changes

### T-069: Release Validation and Launch
- **Dependencies:** T-064 through T-068
- **PRD Sections:** 27, 28
- **Load Agents:** release-validation, ci-devops, security-review
- **Risk Class:** R4
- **Work:**
  - Release checklist: no P0, no data-loss bugs, no destructive-sync bugs, compat corpus passes, credential storage approved, installers validated, docs complete, rollback path exists, crash reporting functional
  - Final smoke test on all 3 platforms
  - Support infrastructure operational
  - Telemetry dashboards live
- **Done When:**
  - Release Validation Agent marks READY FOR GA
  - All launch gate conditions met (Governance Addendum Section 9)
  - v1.0 shipped

---

## Post-Launch: Stabilization

### T-070: Post-Launch Stabilization
- **Dependencies:** T-069
- **PRD Sections:** 28, 32
- **Load Agents:** maintenance-regression, test-and-qa, documentation
- **Risk Class:** R2-R4 (varies by fix)
- **Work:**
  - Hotfix pipeline for critical issues
  - Telemetry-driven prioritization
  - Connector stability patches
  - Performance optimization based on real-world usage
  - Community feedback integration
  - v1.1 stability release
  - v1.5 roadmap planning (mobile companion, QUIC peer, Git awareness, CLI extensions)
- **Done When:**
  - Crash-free rate over 99.9 percent
  - Top 10 community issues resolved
  - v1.1 released
  - v1.5 roadmap published

---

## Task Dependency Map

```
T-001 (Scaffold) --> T-002 (Crates) --> T-003 (SQLite) --> T-004 (State)
T-001 --> T-005 (Design System) --> T-006 (Dual Pane) --> T-007 (File List)
T-007 --> T-008 (Tabs), T-009 (Nav), T-010 (File Ops), T-012 (Search)
T-010 --> T-011 (DnD), T-013 (Context/Keyboard), T-015 (Transfer Queue)
T-015 --> T-016 (Resume), T-017 (Conflicts), T-018 (Verify/History)
T-015 + T-019 --> T-020 to T-030 (All Connectors -- parallelizable)
T-015 + T-025 --> T-031 (Peer), T-032 (Migration), T-033 (S2S)
T-002 --> T-034 (Unicode) --> T-035 (Reserved) --> T-036 (Profiles) --> T-037 (Mapping) --> T-038 (Tiers)
T-015 + T-003 --> T-039 (Sync) --> T-040 (Dry-run), T-041 (Conflicts), T-042 (Verify/Rollback)
T-015 + T-039 --> T-043 (AI) --> T-044 (NL), T-045 (Safety)
T-001 --> T-048 (Admin Shell) --> T-049 (RBAC) --> T-050 (Policy) --> T-051 (Approvals)
All functional --> T-066 (E2E) --> T-067 (Security) --> T-068 (Docs) --> T-069 (Launch)
```

---

**Total: 70 tasks. No sprints. No story points. Just build in order.**


---

## Appendix: Governance Rules Extracted from Senior Dev Epic Framework

The following sections are extracted from the senior developer epic and sprint framework document. They add governance guardrails that complement the 70-task build sequence above.

---

## Definition of Ready (Must Pass Before Any Task Starts)

No task may begin execution unless ALL of the following are true:

1. PRD section is cited
2. Expected behavior is clear
3. Out-of-scope is clear
4. Acceptance criteria exist (the Done When field)
5. Risk class is assigned (R0-R4)
6. Owner agent is identified (the Load Agents field)
7. Required reviewers are known
8. Dependencies are identified and marked DONE
9. Test expectations are defined
10. User-facing copy/doc impact is noted if relevant

If any of these are missing, the task is NOT READY. The Orchestrator Agent must not assign it.

---

## WIP Limits (Prevent Agent Thrash)

Per-execution-cycle limits:
- Maximum 3 active phases being worked simultaneously
- Maximum 10-15 tasks in serious progress across all agents
- Maximum 2 concurrent R4 (destructive/data-integrity) tasks
- Maximum 1 major schema migration task unless the cycle is dedicated to platform changes

Per-agent limits:
- Each primary implementation agent should own no more than 2 major tasks at once
- QA Agent should not be overloaded with more certification work than can be completed in the cycle
- Security Review Agent queue must remain bounded -- do not dump 10 R3/R4 reviews simultaneously

Why this matters: AI agents optimize for throughput. Without WIP limits, agents will start many tasks and finish few, creating integration chaos. Finishing fewer tasks completely is better than starting many tasks partially.

---

## Mandatory Hardening Checkpoints

Reserve dedicated hardening focus at these points in the build sequence. During hardening, the priority shifts from new features to stability, regression, security, and documentation.

Checkpoint 1 -- After Phase 2 (File Manager Core complete)
- After tasks: T-014
- Focus: Local browsing stability, keyboard/accessibility, cross-platform rendering, performance baselines
- Gate: No P0 bugs in file manager. 10K file list performance verified. Accessibility baseline passes.

Checkpoint 2 -- After Phase 5 (All Connectors complete)
- After tasks: T-030
- Focus: Connector auth stability, resume/retry reliability, credential storage security review, connection diagnostics
- Gate: All connectors pass certification. Security review of auth/credential flows complete. No connector crashes on error paths.

Checkpoint 3 -- After Phase 8 (Sync Engine complete)
- After tasks: T-042
- Focus: Sync integrity, destructive-path QA, conflict policy correctness, dry-run accuracy, compatibility engine corpus tests
- Gate: Sync corpus passes. No unresolved destructive-sync bugs. Compatibility engine passes 200+ edge case tests. Rollback behavior verified.

Checkpoint 4 -- After Phase 10 (Enterprise complete)
- After tasks: T-053
- Focus: Tenant isolation, RBAC enforcement, policy propagation, approval workflows, audit immutability, AI safety controls
- Gate: Enterprise QA suite passes. Security review of tenant isolation complete. Admin console authorization verified.

Checkpoint 5 -- Before Launch (Phase 13)
- Before tasks: T-069
- Focus: Full E2E journey validation, security audit, installer verification, documentation completeness, rollback readiness
- Gate: All 10 launch gates from Governance Addendum Section 9 must pass. Release Validation Agent marks READY FOR GA.

Rule: No hardening checkpoint may be skipped. If a checkpoint gate fails, new feature tasks in subsequent phases are blocked until the gate passes.

---

## What NOT To Do (Anti-Patterns for AI Agent Execution)

Do NOT run the build sequence with any of the following patterns:

1. One giant unsliced backlog -- Each task is already sliced. Do not merge them back into vague mega-tasks.

2. No Definition of Done enforcement -- A task is not DONE just because code exists. Tests, QA evidence, docs, rollback notes, and security review (where required) must all be present.

3. Vague agent prompts -- Never prompt an agent with build the sync engine or do security. Each task already specifies exact scope, PRD sections, and acceptance criteria. Use them.

4. No hardening checkpoints -- AI agents create speed but not stability. Without dedicated hardening passes, the codebase will accumulate hidden regressions.

5. No QA independence -- The agent that writes code must not be the sole agent that validates it. The Test and QA Agent must independently verify.

6. No security gate for destructive paths -- Any task touching delete, overwrite, sync conflict resolution, migration, encryption, or credential storage (R3/R4) must pass through Security Review Agent. No exceptions.

7. Working on 10 unrelated tasks simultaneously -- WIP limits exist for a reason. Finish fewer tasks completely rather than starting many partially.

8. Skipping rollback notes -- Every non-trivial task must include rollback guidance. If something breaks after merge, the team must know how to revert.

9. Treating implementation as release readiness -- Code being written does not mean it is launch-ready. Release Validation Agent must independently confirm readiness.

10. Fabricating test evidence -- No agent may claim tests pass without actual execution evidence. No agent may mark a connector as certified without running the certification suite.

---

These governance rules apply to all 70 tasks in the build sequence above.
