# Changelog

All notable changes to the Unified File Operations Platform are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-03-15

### Added

#### Desktop Application
- Tauri 2.0-based cross-platform desktop app (macOS 10.15+, Windows, Linux)
- Dual-pane file manager with tabbed navigation and four view modes (list, detail, grid, compact)
- Simple mode (default) with streamlined sidebar: Files, Transfers, Sync, Cloud & Servers, Search, Favorites, Activity, Settings
- Advanced mode with full feature set: dual panes, command palette, filter bar, data grid, terminal panel
- Instant mode switching between Simple and Advanced without restart
- Onboarding wizard with 6 steps: Welcome, Choose Style, Connect Locations, Quick Start, Compatibility, Ready
- 17 guided flow wizards for common tasks (cloud connections, transfers, sync, backup, migration)
- Activity feed with structured error reporting (what happened, why, what the app did, what user can do)
- Command palette (Ctrl/Cmd+K) for quick access to all features
- Context menu with full file operations
- Virtual scrolling for large directories via @tanstack/react-virtual

#### File Operations
- Copy, move, rename, duplicate, delete, create folder, create file
- Undo support (up to 10 levels) for all file operations
- File metadata inspection
- Breadcrumb navigation with path segments

#### Transfer Engine
- Multi-protocol file transfers with queue management
- Pause, resume, cancel, retry operations
- Priority levels (high, normal, low) with queue reordering
- Bandwidth throttling (global and per-connection)
- Conflict resolution policies (ask, overwrite, skip, rename, newer-wins)
- Three-tier verification: Quick (size), Standard (xxHash3), Full (SHA-256)
- Transfer history with search, export (CSV), and cleanup
- Three-layer journal for crash recovery
- Automatic recovery of interrupted transfers on restart

#### Connectors
- SFTP: SSH-based transfer with key auth, jump hosts, host key verification
- FTP/FTPS: Active/passive mode with TLS support
- WebDAV: HTTP-based DAV with Nextcloud/ownCloud compatibility
- SMB/CIFS: Windows shares and NAS devices with LAN discovery
- NFS: NFSv3/v4 with Unix permission display
- Local Drive: Drive-to-drive transfers with preflight checks
- Amazon S3: S3 and S3-compatible services (MinIO, Wasabi, B2 S3)
- Google Drive: OAuth 2.0, My Drive/Shared Drives, Docs export, versioning
- Dropbox: OAuth 2.0, team/shared folders, session uploads
- OneDrive: Personal/Business, SharePoint, naming restrictions
- Backblaze B2: Native API with large file uploads

#### Connection Management
- Save, test, and organize connections into groups
- Import/export connection profiles (JSON)
- OS keychain credential storage (macOS Keychain, Windows Credential Manager, Linux Secret Service)

#### Sync Engine
- Bidirectional and one-way sync modes
- Manual, watcher (filesystem events), and scheduled (cron) triggers
- Dry run with preview export (CSV)
- Conflict detection and resolution (keep source, keep dest, keep both, skip)
- Quarantine for conflicting files
- Rollback to pre-sync state
- Health indicators (green, yellow, red, gray)
- Sync reports with CSV and JSON export

#### Peer-to-Peer
- mDNS/Bonjour peer discovery on local network
- Trust levels: untrusted, trusted, blocked
- TLS-encrypted peer transfers
- Manual connection via IP address
- Transfer request and approval flow

#### Server-to-Server Transfers
- Direct transfer between remote endpoints
- Capability matrix for transfer method selection
- Pause, cancel, retry support

#### AI Assistant
- Chat interface with contextual assistance
- Error explanation (plain language + causes + fixes)
- Proactive suggestions based on user context
- Natural language job creation ("Sync my Documents to Drive every night")
- Safety controls: confirmation gate for destructive actions
- Full audit logging of all AI interactions
- Feature toggles for granular control
- Local/cloud model routing

#### Terminal
- Built-in local terminal with default shell detection
- Remote SSH terminal via saved connections
- Multiple sessions with tab switching
- Split layouts (horizontal/vertical)
- Path escaping for drag-and-drop

#### Encryption & Security
- Encrypted vaults with AES-256-GCM and ChaCha20-Poly1305
- Argon2id key derivation with zeroize memory safety
- Encrypt-for-upload / decrypt-from-download
- Vault password management
- Transport security checking and HTTPS enforcement
- Encryption policies (per file type, per destination)

#### Cross-Platform Compatibility
- Compatibility engine for file name validation across platforms
- Detection of Windows reserved names, invalid characters, path length limits
- Unicode normalization (NFC/NFD) handling
- Safe auto-renaming with undo capability
- Compatibility badges on files

#### Batch Rename
- Pattern-based renaming with tokens ({name}, {ext}, {num}, {date}, {parent}, {counter})
- Find/replace (literal and regex)
- Case transforms (upper, lower, title, sentence)
- Preview before apply
- Batch undo

#### Preview Engine
- File preview pane with EXIF metadata for images
- Text file preview
- Multiple file format support

#### Archive Tools
- Browse, create, extract archives
- ZIP, TAR, TAR.GZ, 7Z format support
- Password-protected archives
- Configurable compression levels

#### Integrity Tools
- Checksum computation and verification (MD5, SHA-1, SHA-256)
- Duplicate file finder with resolution actions
- Tags and color labels
- Smart folders with dynamic rules

#### Themes & Accessibility
- Four themes: System (auto), Light, Dark, High Contrast
- Instant theme switching via CSS custom properties
- Full keyboard navigation with visible focus indicators
- ARIA labels and screen reader support
- Design token system for consistent styling

#### Admin Console
- Next.js web application for organizational administration
- Dashboard with stats cards and overview panels
- User management with role-based access (Super Admin, Org Admin, Manager, User, Viewer)
- Device fleet management with policy compliance tracking
- Policy engine with enforce/warn/audit modes
- Approval workflows for sensitive operations
- Tamper-evident audit log with cryptographic chaining
- Connector management (organization-wide)
- Workspace management
- AI governance settings and audit
- Billing page
- OpenAPI 3.0 specification at /api/openapi
- REST API for all admin operations
- Webhook subscriptions for event notifications

#### CLI
- Rust-based CLI (`ufop`) with human/JSON/YAML output formats
- Commands: login, connection (list/add/test/remove), transfer, sync (list/create/run/delete), compat, checksum, duplicates, rename, archive (create/extract/list), status
- Dry-run mode for safe previewing
- Bandwidth limiting
- Configurable verbosity levels
- Exit codes for scripting

#### Design System
- Shared design tokens package (colors, typography, spacing, shadows, radii, animations)
- Shared UI components package (button, dialog, dropdown, input, badge, tabs, tooltip, scroll area, separator)

#### CI/CD
- GitHub Actions CI pipeline with lint, type check, test, format verification
- Supports both frontend (TypeScript/React) and backend (Rust) validation
- Concurrency groups to cancel superseded CI runs

### Security
- OS keychain integration for credential storage
- TLS enforcement for all supported protocols
- CSP configuration for the WebView
- Tauri plugin-based file system access
- Audit logging for all operations
- AI action confirmation gate for destructive operations
