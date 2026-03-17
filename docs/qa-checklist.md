# Unified File Operations Platform -- V8-Compatible QA Checklist

**Version:** 1.0.0
**Generated:** 2026-03-16
**Platform:** Tauri 2 Desktop (React/Vite) + Next.js Admin Console
**Total Items:** 500+

---

## Table of Contents

1. [Blocker Registry](#blocker-registry)
2. [Pattern Registry](#pattern-registry)
3. [Section 1: Authentication & Authorization](#section-1-authentication--authorization)
4. [Section 2: Desktop App -- Core Navigation & Layout](#section-2-desktop-app----core-navigation--layout)
5. [Section 3: Desktop App -- File Management](#section-3-desktop-app----file-management)
6. [Section 4: Desktop App -- Transfer Engine](#section-4-desktop-app----transfer-engine)
7. [Section 5: Desktop App -- Connection & Connector Management](#section-5-desktop-app----connection--connector-management)
8. [Section 6: Desktop App -- Sync Engine](#section-6-desktop-app----sync-engine)
9. [Section 7: Desktop App -- Peer & Server Transfer](#section-7-desktop-app----peer--server-transfer)
10. [Section 8: Desktop App -- AI Assistant](#section-8-desktop-app----ai-assistant)
11. [Section 9: Desktop App -- Terminal Integration](#section-9-desktop-app----terminal-integration)
12. [Section 10: Desktop App -- Security & Encryption](#section-10-desktop-app----security--encryption)
13. [Section 11: Desktop App -- Preview, Archive, Batch Rename, Integrity](#section-11-desktop-app----preview-archive-batch-rename-integrity)
14. [Section 12: Desktop App -- Onboarding, Guided Flows, Simple Mode](#section-12-desktop-app----onboarding-guided-flows-simple-mode)
15. [Section 13: Desktop App -- Theme, i18n, Accessibility](#section-13-desktop-app----theme-i18n-accessibility)
16. [Section 14: Admin Console -- Login & Dashboard](#section-14-admin-console----login--dashboard)
17. [Section 15: Admin Console -- Users & RBAC](#section-15-admin-console----users--rbac)
18. [Section 16: Admin Console -- Devices](#section-16-admin-console----devices)
19. [Section 17: Admin Console -- Policy Engine](#section-17-admin-console----policy-engine)
20. [Section 18: Admin Console -- Approval Workflows](#section-18-admin-console----approval-workflows)
21. [Section 19: Admin Console -- Audit Explorer](#section-19-admin-console----audit-explorer)
22. [Section 20: Admin Console -- Connectors](#section-20-admin-console----connectors)
23. [Section 21: Admin Console -- AI Governance](#section-21-admin-console----ai-governance)
24. [Section 22: Admin Console -- Billing](#section-22-admin-console----billing)
25. [Section 23: Admin Console -- Shared Workspaces](#section-23-admin-console----shared-workspaces)
26. [Section 24: Responsive & Viewport Testing](#section-24-responsive--viewport-testing)
27. [Section 25: Performance & Stress Testing](#section-25-performance--stress-testing)
28. [Section 26: Cross-Platform & Build Verification](#section-26-cross-platform--build-verification)
29. [Section 27: Persistence & Crash Recovery](#section-27-persistence--crash-recovery)
30. [Section 28: Auto-Update](#section-28-auto-update)

---

## Blocker Registry

| Blocker ID | Description | Severity | Section | Status | Date Found | Date Resolved |
|------------|-------------|----------|---------|--------|------------|---------------|
| BLK-001 | _template_ | P0/P1/P2 | CL-xxx | Open/Closed | YYYY-MM-DD | YYYY-MM-DD |

---

## Pattern Registry

| Pattern ID | Pattern Group | Description | Applies To |
|------------|--------------|-------------|------------|
| PG-FORM | Form Validation | Required fields, min/max, type coercion, XSS, SQL injection, unicode, disabled submit, error clear-on-fix | All forms |
| PG-TABLE | Table Interactions | Sort asc/desc, filter, search, pagination, empty state, loading skeleton, column resize, row click | All tables |
| PG-MODAL | Modal/Dialog | Open, close via X, close via Escape, close via overlay, focus trap, scroll lock, nested modals, loading state | All dialogs |
| PG-TOAST | Toast/Notification | Appear, auto-dismiss, manual dismiss, stack multiple, action buttons, accessibility announcement | All notifications |
| PG-NAV | Navigation | Route load, back/forward, deep link, 404 fallback, active state highlight, breadcrumb sync | All routes |
| PG-DND | Drag and Drop | Initiate drag, drop valid target, drop invalid target, cancel drag, visual feedback, accessibility alternative | File list, tabs |
| PG-KBD | Keyboard Navigation | Tab order, Enter/Space activation, Escape dismiss, arrow key navigation, shortcut conflicts, focus visible | All interactive elements |
| PG-EMPTY | Empty States | First-use empty, search-no-results empty, error empty, action CTA in empty state | All lists/tables |
| PG-LOAD | Loading States | Skeleton/spinner on load, error fallback on fail, retry action, optimistic update rollback | All async operations |
| PG-CONFIRM | Destructive Confirmation | Confirm dialog before delete/overwrite, cancel returns to prior state, no double-submit | All destructive ops |

---

## Section 1: Authentication & Authorization

### 1.1 Admin Console Login (`{{BACKEND_URL}}/login`)

- [ ] **CL-001** [SMOKE] [PATTERN-GROUP: PG-FORM] -- Navigate to `{{BACKEND_URL}}/login`, verify login form renders with Email, Password fields, Sign In button, and SSO button
- [ ] **CL-002** [SMOKE] [PATTERN-GROUP: PG-FORM] -- Submit valid email + password, verify redirect to `{{BACKEND_URL}}/dashboard`
- [ ] **CL-003** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit empty email + empty password, verify inline error "Please enter both email and password"
- [ ] **CL-004** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit valid email + empty password, verify error displayed
- [ ] **CL-005** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit empty email + valid password, verify error displayed
- [ ] **CL-006** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit email with XSS payload `<script>alert(1)</script>@test.com`, verify sanitized / error
- [ ] **CL-007** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit email with SQL injection `'; DROP TABLE users;--@test.com`, verify no SQL error
- [ ] **CL-008** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit email with unicode `test@example.com` using RTL override characters, verify handled
- [ ] **CL-009** [DEEP] [PATTERN-GROUP: PG-FORM] -- Click "Sign in with SSO (SAML)", verify error message about SSO configuration
- [ ] **CL-010** [DEEP] [PATTERN-GROUP: PG-NAV] -- When already authenticated, navigate to `/login`, verify redirect to `/dashboard`

### 1.2 Auth State Persistence

- [ ] **CL-011** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- After login, refresh the page, verify user stays authenticated
- [ ] **CL-012** [DEEP] [PATTERN-GROUP: PG-NAV] -- Without authentication, navigate to `{{BACKEND_URL}}/dashboard`, verify redirect to `/login`
- [ ] **CL-013** [DEEP] [PATTERN-GROUP: PG-NAV] -- Without authentication, navigate to `{{BACKEND_URL}}/users`, verify redirect to `/login`
- [ ] **CL-014** [DEEP] [PATTERN-GROUP: PG-NAV] -- Without authentication, navigate to `{{BACKEND_URL}}/policies`, verify redirect to `/login`

---

## Section 2: Desktop App -- Core Navigation & Layout

### 2.1 App Launch & Initial State

- [ ] **CL-015** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Launch desktop app, verify main window opens at 1280x800 with title "Unified File Operations Platform"
- [ ] **CL-016** [SMOKE] [PATTERN-GROUP: PG-NAV] -- Verify app loads at `{{FRONTEND_URL}}/` (Home route) with SimpleModeWrapper
- [ ] **CL-017** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Verify FileManager component renders with toolbar, sidebar, file pane, and status bar
- [ ] **CL-018** [DEEP] [PATTERN-GROUP: PG-NAV] -- Navigate to `{{FRONTEND_URL}}/demo`, verify DemoPage renders
- [ ] **CL-019** [DEEP] [PATTERN-GROUP: PG-NAV] -- Navigate to `{{FRONTEND_URL}}/nonexistent`, verify redirect to `/` (catch-all)

### 2.2 Toolbar

- [ ] **CL-020** [SMOKE] [PATTERN-GROUP: PG-KBD] -- Verify toolbar displays: sidebar toggle, app title, mode badge, view mode selector, undo button (when stack non-empty), AI toggle, terminal toggle, theme switcher
- [ ] **CL-021** [SMOKE] [PATTERN-GROUP: PG-KBD] -- Click sidebar toggle button, verify sidebar hides; click again, verify sidebar shows
- [ ] **CL-022** [DEEP] [PATTERN-GROUP: PG-KBD] -- Press `Cmd+B` (macOS) / `Ctrl+B` (Win/Linux), verify sidebar toggles
- [ ] **CL-023** [DEEP] [PATTERN-GROUP: PG-KBD] -- Press `Cmd+Shift+A`, verify AI panel toggles open/closed
- [ ] **CL-024** [DEEP] [PATTERN-GROUP: PG-KBD] -- Press `` Cmd+` ``, verify terminal panel toggles open/closed
- [ ] **CL-025** [DEEP] [PATTERN-GROUP: PG-KBD] -- Press `Cmd+Z` when undo stack has entries, verify last operation undone

### 2.3 View Mode Selector

- [ ] **CL-026** [SMOKE] [PATTERN-GROUP: PG-KBD] -- Verify 4 view mode buttons: List, Detail, Grid, Compact
- [ ] **CL-027** [SMOKE] [PATTERN-GROUP: PG-KBD] -- Click each view mode button, verify active state highlights and file list view changes
- [ ] **CL-028** [DEEP] [PATTERN-GROUP: PG-KBD] -- Verify `aria-pressed` attribute toggles correctly on each view mode button
- [ ] **CL-029** [DEEP] [PATTERN-GROUP: PG-KBD] -- Verify view mode persists per-pane (pane 0 can be "list", pane 1 can be "grid")

### 2.4 Dual Pane Layout

- [ ] **CL-030** [SMOKE] [PATTERN-GROUP: PG-NAV] -- Verify app starts in single-pane mode (default for Simple mode)
- [ ] **CL-031** [DEEP] [PATTERN-GROUP: PG-NAV] -- Toggle dual-pane mode, verify two panes appear side by side with resizable divider
- [ ] **CL-032** [DEEP] [PATTERN-GROUP: PG-DND] -- Drag the pane divider, verify split percentage changes (clamped 20-80%)
- [ ] **CL-033** [DEEP] [PATTERN-GROUP: PG-KBD] -- Click in left pane then right pane, verify active pane indicator switches
- [ ] **CL-034** [DEEP] [PATTERN-GROUP: PG-NAV] -- Navigate to different directories in each pane, verify they are independent

### 2.5 Sidebar Navigation

- [ ] **CL-035** [SMOKE] [PATTERN-GROUP: PG-NAV] -- Verify sidebar shows: tree view, favorites section, recent locations
- [ ] **CL-036** [SMOKE] [PATTERN-GROUP: PG-NAV] -- Click a tree node, verify active pane navigates to that path
- [ ] **CL-037** [DEEP] [PATTERN-GROUP: PG-NAV] -- Click expand arrow on tree node, verify children load
- [ ] **CL-038** [DEEP] [PATTERN-GROUP: PG-NAV] -- Add a favorite via context menu, verify it appears in Favorites section
- [ ] **CL-039** [DEEP] [PATTERN-GROUP: PG-NAV] -- Remove a favorite, verify it disappears from Favorites section
- [ ] **CL-040** [DEEP] [PATTERN-GROUP: PG-NAV] -- Verify recent locations list updates when navigating directories
- [ ] **CL-041** [DEEP] [PATTERN-GROUP: PG-NAV] -- Verify sidebar width is resizable and persists (min 180, max 480)

### 2.6 Tab Bar

- [ ] **CL-042** [SMOKE] [PATTERN-GROUP: PG-NAV] -- Verify at least one tab is always open per pane
- [ ] **CL-043** [SMOKE] [PATTERN-GROUP: PG-NAV] -- Open a new tab, verify it appears and becomes active
- [ ] **CL-044** [DEEP] [PATTERN-GROUP: PG-NAV] -- Close a tab (not the last one), verify it closes and adjacent tab activates
- [ ] **CL-045** [DEEP] [PATTERN-GROUP: PG-NAV] -- Try to close the last tab, verify it is prevented (at least 1 tab required)
- [ ] **CL-046** [DEEP] [PATTERN-GROUP: PG-DND] -- Drag-reorder tabs, verify new order persists
- [ ] **CL-047** [DEEP] [PATTERN-GROUP: PG-NAV] -- Pin a tab, verify pinned indicator appears and tab cannot be closed
- [ ] **CL-048** [DEEP] [PATTERN-GROUP: PG-NAV] -- Unpin a tab, verify it becomes closable again
- [ ] **CL-049** [DEEP] [PATTERN-GROUP: PG-NAV] -- Click a non-active tab, verify it becomes the active tab and file list updates

### 2.7 Breadcrumb Bar

- [ ] **CL-050** [SMOKE] [PATTERN-GROUP: PG-NAV] -- Verify breadcrumb displays current path segments
- [ ] **CL-051** [SMOKE] [PATTERN-GROUP: PG-NAV] -- Click a breadcrumb segment, verify navigation to that path
- [ ] **CL-052** [DEEP] [PATTERN-GROUP: PG-NAV] -- Navigate deep (5+ levels), verify breadcrumb shows all segments or truncates with "..."
- [ ] **CL-053** [DEEP] [PATTERN-GROUP: PG-NAV] -- Click root breadcrumb, verify navigation to "/"

### 2.8 Status Bar

- [ ] **CL-054** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Verify status bar shows "Ready" when no selection
- [ ] **CL-055** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Select 1 file, verify status bar shows "1 item selected"
- [ ] **CL-056** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Select 5 files, verify status bar shows "5 items selected"
- [ ] **CL-057** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify version number "UFOP v0.1.0" displayed in status bar

### 2.9 Command Palette

- [ ] **CL-058** [SMOKE] [PATTERN-GROUP: PG-MODAL] -- Press `Cmd+K`, verify command palette opens
- [ ] **CL-059** [SMOKE] [PATTERN-GROUP: PG-MODAL] -- Type a command name, verify fuzzy search filters results
- [ ] **CL-060** [DEEP] [PATTERN-GROUP: PG-KBD] -- Use arrow keys to navigate commands, press Enter to execute
- [ ] **CL-061** [DEEP] [PATTERN-GROUP: PG-MODAL] -- Press `Escape`, verify command palette closes
- [ ] **CL-062** [DEEP] [PATTERN-GROUP: PG-KBD] -- Execute "Toggle Dual Pane" command, verify pane mode toggles
- [ ] **CL-063** [DEEP] [PATTERN-GROUP: PG-KBD] -- Execute "Set Theme" command, verify theme changes
- [ ] **CL-064** [DEEP] [PATTERN-GROUP: PG-KBD] -- Execute "Set View Mode" command, verify view mode changes

---

## Section 3: Desktop App -- File Management

### 3.1 File List Display

- [ ] **CL-065** [SMOKE] [PATTERN-GROUP: PG-TABLE] -- Verify file list displays files and folders with name, size, modified date
- [ ] **CL-066** [SMOKE] [PATTERN-GROUP: PG-TABLE] -- Verify folders sort before files by default
- [ ] **CL-067** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Click column header "Name", verify sort toggles asc/desc
- [ ] **CL-068** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Click column header "Size", verify sort by file size
- [ ] **CL-069** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Click column header "Modified", verify sort by date
- [ ] **CL-070** [DEEP] [PATTERN-GROUP: PG-EMPTY] -- Navigate to an empty directory, verify empty state message

### 3.2 File Selection

- [ ] **CL-071** [SMOKE] [PATTERN-GROUP: PG-KBD] -- Click a file, verify single selection (highlight)
- [ ] **CL-072** [SMOKE] [PATTERN-GROUP: PG-KBD] -- Cmd+Click multiple files, verify multi-select
- [ ] **CL-073** [DEEP] [PATTERN-GROUP: PG-KBD] -- Shift+Click to select a range, verify contiguous range selected
- [ ] **CL-074** [DEEP] [PATTERN-GROUP: PG-KBD] -- Cmd+A, verify all files in current directory selected
- [ ] **CL-075** [DEEP] [PATTERN-GROUP: PG-KBD] -- After selecting, click empty area, verify selection clears

### 3.3 File Navigation

- [ ] **CL-076** [SMOKE] [PATTERN-GROUP: PG-NAV] -- Double-click a folder, verify navigation into that folder
- [ ] **CL-077** [DEEP] [PATTERN-GROUP: PG-NAV] -- Double-click a file, verify it opens with system default application
- [ ] **CL-078** [DEEP] [PATTERN-GROUP: PG-KBD] -- Use arrow keys to move focus between files
- [ ] **CL-079** [DEEP] [PATTERN-GROUP: PG-KBD] -- Press Enter on focused folder, verify navigation into folder

### 3.4 File Operations (Tauri IPC)

- [ ] **CL-080** [SMOKE] [PATTERN-GROUP: PG-CONFIRM] -- Copy files: select files, invoke copy, paste in different directory, verify files copied
- [ ] **CL-081** [SMOKE] [PATTERN-GROUP: PG-CONFIRM] -- Move files: select files, invoke move, verify files moved and removed from source
- [ ] **CL-082** [SMOKE] [PATTERN-GROUP: PG-CONFIRM] -- Rename file: right-click > Rename, enter new name, verify renamed
- [ ] **CL-083** [SMOKE] [PATTERN-GROUP: PG-CONFIRM] -- Delete files: select files, invoke delete, confirm dialog, verify deleted
- [ ] **CL-084** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Duplicate files: select files, invoke duplicate, verify `file (copy)` created
- [ ] **CL-085** [DEEP] [PATTERN-GROUP: PG-FORM] -- Create new folder: right-click > New Folder, enter name, verify created
- [ ] **CL-086** [DEEP] [PATTERN-GROUP: PG-FORM] -- Create new file: right-click > New File, enter name, verify created
- [ ] **CL-087** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Undo delete operation, verify file restored
- [ ] **CL-088** [DEEP] [PATTERN-GROUP: PG-FORM] -- Rename to empty string, verify error / prevented
- [ ] **CL-089** [DEEP] [PATTERN-GROUP: PG-FORM] -- Rename with invalid characters (`/`, `\0`), verify error
- [ ] **CL-090** [DEEP] [PATTERN-GROUP: PG-FORM] -- Rename to existing filename, verify conflict resolution dialog
- [ ] **CL-091** [DEEP] [PATTERN-GROUP: PG-FORM] -- Create folder with name containing unicode (CJK, Arabic, emoji), verify created
- [ ] **CL-092** [DEEP] [PATTERN-GROUP: PG-FORM] -- Create folder with 255-char name (max path), verify handled
- [ ] **CL-093** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Get file metadata: right-click > Get Info, verify size, permissions, dates shown

### 3.5 Filter Bar

- [ ] **CL-094** [SMOKE] [PATTERN-GROUP: PG-TABLE] -- Type in filter input, verify file list filters in real-time
- [ ] **CL-095** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Filter for "*.ts", verify only TypeScript files shown
- [ ] **CL-096** [DEEP] [PATTERN-GROUP: PG-EMPTY] -- Filter for non-existent pattern, verify "0 of N" count and empty state
- [ ] **CL-097** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Clear filter, verify all files shown again

### 3.6 Context Menu

- [ ] **CL-098** [SMOKE] [PATTERN-GROUP: PG-KBD] -- Right-click a file, verify context menu appears at cursor position
- [ ] **CL-099** [SMOKE] [PATTERN-GROUP: PG-KBD] -- Verify context menu items: Open, Copy, Cut, Paste, Delete, Rename, Duplicate, New Folder, New File, Add to Favorites, Get Info, Refresh, Select All, Invert Selection
- [ ] **CL-100** [DEEP] [PATTERN-GROUP: PG-KBD] -- Right-click a directory, verify "Open in Terminal" option appears
- [ ] **CL-101** [DEEP] [PATTERN-GROUP: PG-KBD] -- Verify "Explain Error" option in context menu (AI integration)
- [ ] **CL-102** [DEEP] [PATTERN-GROUP: PG-MODAL] -- Click away from context menu, verify it closes
- [ ] **CL-103** [DEEP] [PATTERN-GROUP: PG-KBD] -- Press Escape, verify context menu closes

### 3.7 Drag and Drop

- [ ] **CL-104** [SMOKE] [PATTERN-GROUP: PG-DND] -- Drag files from one pane to another, verify move/copy operation
- [ ] **CL-105** [DEEP] [PATTERN-GROUP: PG-DND] -- Drag files onto a folder in the same pane, verify move into folder
- [ ] **CL-106** [DEEP] [PATTERN-GROUP: PG-DND] -- Drag files and drop on invalid target, verify no operation
- [ ] **CL-107** [DEEP] [PATTERN-GROUP: PG-DND] -- Verify drag ghost/preview shows file count and names

---

## Section 4: Desktop App -- Transfer Engine

### 4.1 Transfer Queue Management

- [ ] **CL-108** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Enqueue a transfer, verify it appears in transfer panel with status "Queued"
- [ ] **CL-109** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Verify transfer shows source path, destination path, file size, and progress
- [ ] **CL-110** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Enqueue multiple transfers, verify queue ordering
- [ ] **CL-111** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Set transfer priority, verify reordering in queue
- [ ] **CL-112** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Reorder transfer via drag or API, verify new position

### 4.2 Transfer Controls

- [ ] **CL-113** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Pause an active transfer, verify status changes to "Paused"
- [ ] **CL-114** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Resume a paused transfer, verify status changes to "Active"
- [ ] **CL-115** [SMOKE] [PATTERN-GROUP: PG-CONFIRM] -- Cancel an active transfer, verify status changes to "Cancelled"
- [ ] **CL-116** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify progress updates (bytes transferred, percentage, speed)
- [ ] **CL-117** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Complete a transfer, verify status changes to "Completed"
- [ ] **CL-118** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Fail a transfer, verify status changes to "Failed" with error message

### 4.3 Retry Logic

- [ ] **CL-119** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Retry a single failed transfer, verify it re-enters queue
- [ ] **CL-120** [DEEP] [PATTERN-GROUP: PG-LOAD] -- "Retry All Failed" button, verify all failed transfers re-queued
- [ ] **CL-121** [DEEP] [PATTERN-GROUP: PG-LOAD] -- List failed transfers only, verify filter works
- [ ] **CL-122** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Set custom retry policy (max retries, backoff), verify applied

### 4.4 Conflict Resolution

- [ ] **CL-123** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Transfer to destination with same-name file, verify conflict dialog
- [ ] **CL-124** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Resolve conflict: Overwrite, verify file replaced
- [ ] **CL-125** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Resolve conflict: Skip, verify file skipped
- [ ] **CL-126** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Resolve conflict: Rename, verify auto-rename applied
- [ ] **CL-127** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Set global conflict policy, verify it applies to subsequent transfers
- [ ] **CL-128** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Set per-transfer conflict policy, verify it overrides global

### 4.5 Verification Tiers

- [ ] **CL-129** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify transfer with Tier 1 (size-only), verify size check
- [ ] **CL-130** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify transfer with Tier 2 (checksum), verify hash comparison
- [ ] **CL-131** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Set verification tier, verify `get_verify_tier` returns correct tier
- [ ] **CL-132** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Compute checksum for a file, verify correct hash returned

### 4.6 Transfer History

- [ ] **CL-133** [SMOKE] [PATTERN-GROUP: PG-TABLE] -- Open transfer history panel, verify completed transfers listed
- [ ] **CL-134** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Search transfer history by filename/path, verify results filter
- [ ] **CL-135** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Export transfer history (CSV/JSON), verify file downloads
- [ ] **CL-136** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Cleanup old transfer history, verify entries removed

### 4.7 Throttle Configuration

- [ ] **CL-137** [DEEP] [PATTERN-GROUP: PG-FORM] -- Set global bandwidth throttle, verify transfers respect limit
- [ ] **CL-138** [DEEP] [PATTERN-GROUP: PG-FORM] -- Set per-connection throttle, verify applies to specific connection only
- [ ] **CL-139** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Get throttle config, verify current settings returned

### 4.8 Transfer Panel UI

- [ ] **CL-140** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Verify TransferPanel renders in mini mode (collapsed)
- [ ] **CL-141** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Expand TransferPanel to full mode, verify detailed transfer list
- [ ] **CL-142** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Toggle between mini and full mode, verify smooth transition

---

## Section 5: Desktop App -- Connection & Connector Management

### 5.1 Connection Panel

- [ ] **CL-143** [SMOKE] [PATTERN-GROUP: PG-FORM] -- Open ConnectionPanel, verify quick-connect form renders
- [ ] **CL-144** [SMOKE] [PATTERN-GROUP: PG-FORM] -- Fill quick-connect form (protocol, host, port, user, password), submit, verify connection saved
- [ ] **CL-145** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with empty host, verify error
- [ ] **CL-146** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with invalid port (e.g. "abc"), verify error
- [ ] **CL-147** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with port > 65535, verify error
- [ ] **CL-148** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with empty username for SFTP, verify error
- [ ] **CL-149** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with special characters in password, verify handled
- [ ] **CL-150** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with Unicode hostname, verify handled
- [ ] **CL-151** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with extremely long hostname (256+ chars), verify error

### 5.2 Connection CRUD

- [ ] **CL-152** [SMOKE] [PATTERN-GROUP: PG-TABLE] -- List saved connections, verify table with name, protocol, host
- [ ] **CL-153** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Search connections by name/host, verify filtering
- [ ] **CL-154** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Delete a connection, verify removal
- [ ] **CL-155** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Test connection, verify success/failure indicator
- [ ] **CL-156** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Edit connection details, save, verify updated

### 5.3 Connection Groups

- [ ] **CL-157** [DEEP] [PATTERN-GROUP: PG-FORM] -- Create connection group, verify it appears in group list
- [ ] **CL-158** [DEEP] [PATTERN-GROUP: PG-TABLE] -- List connection groups, verify group names
- [ ] **CL-159** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Delete connection group, verify removal

### 5.4 Import/Export Connections

- [ ] **CL-160** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Export connections, verify JSON file downloaded
- [ ] **CL-161** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Import connections from JSON, verify connections added

### 5.5 Protocol Connectors

- [ ] **CL-162** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Connect via SFTP connector, verify connected state [REQUIRES: SFTP server]
- [ ] **CL-163** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- List remote directory via SFTP, verify file listing [REQUIRES: SFTP server]
- [ ] **CL-164** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Connect via FTP connector [REQUIRES: FTP server]
- [ ] **CL-165** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Connect via FTPS connector (TLS) [REQUIRES: FTPS server]
- [ ] **CL-166** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Connect via WebDAV connector [REQUIRES: WebDAV server]
- [ ] **CL-167** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Connect via SMB connector, discover shares [REQUIRES: SMB/LAN]
- [ ] **CL-168** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Connect via NFS connector, list exports [REQUIRES: NFS server]
- [ ] **CL-169** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Connect via S3 connector [REQUIRES: S3/MinIO]
- [ ] **CL-170** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Connect via Google Drive connector [REQUIRES: Google OAuth]
- [ ] **CL-171** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Connect via Dropbox connector [REQUIRES: Dropbox OAuth]
- [ ] **CL-172** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Connect via OneDrive connector [REQUIRES: OneDrive OAuth]
- [ ] **CL-173** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Connect via Backblaze B2 connector [REQUIRES: B2 account]
- [ ] **CL-174** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Disconnect from active connector, verify disconnected state
- [ ] **CL-175** [DEEP] [PATTERN-GROUP: PG-LOAD] -- List all registered protocols, verify 12 protocols returned

### 5.6 Drive-to-Drive

- [ ] **CL-176** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Detect connected drives, verify list of drives with name, type, filesystem
- [ ] **CL-177** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Run transfer preflight check between drives, verify space/compatibility report
- [ ] **CL-178** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Eject a drive, verify safe eject performed [REQUIRES: External drive]

### 5.7 Device Sidebar

- [ ] **CL-179** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Open DeviceSidebar, verify detected drives shown
- [ ] **CL-180** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Click a drive, verify navigation to drive root
- [ ] **CL-181** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Click eject button on drive, verify eject dialog

---

## Section 6: Desktop App -- Sync Engine

### 6.1 Sync Pair Management

- [ ] **CL-182** [SMOKE] [PATTERN-GROUP: PG-FORM] -- Open SyncPanel, verify sync pair list renders
- [ ] **CL-183** [SMOKE] [PATTERN-GROUP: PG-FORM] -- Create new sync pair: name, source, dest, mode (mirror/two-way/one-way/incremental), verify created
- [ ] **CL-184** [DEEP] [PATTERN-GROUP: PG-FORM] -- Create sync pair with empty name, verify error
- [ ] **CL-185** [DEEP] [PATTERN-GROUP: PG-FORM] -- Create sync pair with same source and dest, verify error
- [ ] **CL-186** [DEEP] [PATTERN-GROUP: PG-FORM] -- Create sync pair with non-existent path, verify error
- [ ] **CL-187** [DEEP] [PATTERN-GROUP: PG-FORM] -- Create sync pair with unicode path, verify handled
- [ ] **CL-188** [DEEP] [PATTERN-GROUP: PG-FORM] -- Create sync pair with path > 260 chars (Windows limit), verify warning
- [ ] **CL-189** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Delete a sync pair, verify removal
- [ ] **CL-190** [DEEP] [PATTERN-GROUP: PG-FORM] -- Update sync pair settings, verify changes saved

### 6.2 Sync Execution

- [ ] **CL-191** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Run sync on a pair, verify sync report generated
- [ ] **CL-192** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify sync report shows: files added, modified, deleted, skipped, conflicts, errors, bytes
- [ ] **CL-193** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify sync health indicator (green/yellow/red/gray)

### 6.3 Dry Run

- [ ] **CL-194** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Run dry-run on a sync pair, verify preview generated without changes
- [ ] **CL-195** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Verify dry-run preview shows additions, modifications, deletions, skipped, conflicts
- [ ] **CL-196** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Export dry-run preview as CSV, verify file downloads

### 6.4 Conflict Resolution

- [ ] **CL-197** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Trigger sync conflict, verify conflict appears in pending list
- [ ] **CL-198** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Resolve conflict: keep-source, verify source version kept
- [ ] **CL-199** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Resolve conflict: keep-dest, verify destination version kept
- [ ] **CL-200** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Resolve conflict: keep-newer, verify newer file kept
- [ ] **CL-201** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Resolve conflict: keep-both, verify both versions preserved

### 6.5 Rollback

- [ ] **CL-202** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Rollback last sync, verify files restored from quarantine
- [ ] **CL-203** [DEEP] [PATTERN-GROUP: PG-TABLE] -- View quarantine entries, verify list of quarantined files

### 6.6 Sync Watcher

- [ ] **CL-204** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Start sync watcher, verify file system events trigger sync
- [ ] **CL-205** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Stop sync watcher, verify events no longer trigger sync

### 6.7 Sync Reports

- [ ] **CL-206** [DEEP] [PATTERN-GROUP: PG-TABLE] -- View sync reports for a pair, verify historical reports listed
- [ ] **CL-207** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Export sync report as CSV, verify file downloads
- [ ] **CL-208** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Export sync report as JSON, verify file downloads

### 6.8 Cron Schedule

- [ ] **CL-209** [DEEP] [PATTERN-GROUP: PG-FORM] -- Create sync pair with cron schedule, verify `validate_sync_cron` accepts
- [ ] **CL-210** [DEEP] [PATTERN-GROUP: PG-FORM] -- Enter invalid cron expression, verify validation error

---

## Section 7: Desktop App -- Peer & Server Transfer

### 7.1 Peer Discovery

- [ ] **CL-211** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Start peer discovery (mDNS), verify discovery active [REQUIRES: LAN peers]
- [ ] **CL-212** [DEEP] [PATTERN-GROUP: PG-LOAD] -- List discovered peers, verify peer names and addresses
- [ ] **CL-213** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Stop peer discovery, verify stopped
- [ ] **CL-214** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Check if currently discovering, verify boolean status

### 7.2 Peer Trust

- [ ] **CL-215** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Set peer trust level (trusted/untrusted), verify saved
- [ ] **CL-216** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Save a peer to favorites, verify persistence
- [ ] **CL-217** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Remove a saved peer, verify removed
- [ ] **CL-218** [DEEP] [PATTERN-GROUP: PG-FORM] -- Set display name for a peer, verify updated
- [ ] **CL-219** [DEEP] [PATTERN-GROUP: PG-FORM] -- Connect to peer manually by IP, verify connection

### 7.3 Peer Transfer

- [ ] **CL-220** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Request file transfer to peer, verify request sent [REQUIRES: LAN peer]
- [ ] **CL-221** [DEEP] [PATTERN-GROUP: PG-LOAD] -- List pending transfer requests, verify requests shown
- [ ] **CL-222** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Accept/reject incoming transfer request
- [ ] **CL-223** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Monitor transfer progress, verify percentage updates
- [ ] **CL-224** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Cancel peer transfer, verify cancelled

### 7.4 Peer Sidebar

- [ ] **CL-225** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Open PeerSidebar, verify discovered peers listed
- [ ] **CL-226** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify online/offline indicator per peer
- [ ] **CL-227** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Click a peer, verify details panel shows

### 7.5 Server-to-Server Transfer

- [ ] **CL-228** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Preview transfer method between two servers [REQUIRES: Two remote connections]
- [ ] **CL-229** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Create server-to-server transfer, verify created
- [ ] **CL-230** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Start server-to-server transfer, verify progress
- [ ] **CL-231** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Pause/resume/cancel server transfer
- [ ] **CL-232** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Get capability matrix for server-to-server, verify protocols listed

---

## Section 8: Desktop App -- AI Assistant

### 8.1 AI Panel

- [ ] **CL-233** [SMOKE] [PATTERN-GROUP: PG-MODAL] -- Toggle AI panel via toolbar button, verify panel opens/closes
- [ ] **CL-234** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Verify AI panel has 3 tabs: Chat, Suggestions, Audit
- [ ] **CL-235** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify panel width is appropriate and does not overlap file list

### 8.2 AI Chat

- [ ] **CL-236** [SMOKE] [PATTERN-GROUP: PG-FORM] -- Send a chat message, verify user message appears optimistically
- [ ] **CL-237** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Verify assistant response appears after IPC call
- [ ] **CL-238** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Send message when backend is unavailable, verify fallback error message
- [ ] **CL-239** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Load chat history, verify previous messages restored
- [ ] **CL-240** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Clear chat, verify all messages removed
- [ ] **CL-241** [DEEP] [PATTERN-GROUP: PG-FORM] -- Send empty message, verify prevented or handled
- [ ] **CL-242** [DEEP] [PATTERN-GROUP: PG-FORM] -- Send message with XSS payload, verify sanitized display
- [ ] **CL-243** [DEEP] [PATTERN-GROUP: PG-FORM] -- Send message with 10,000+ characters, verify handled
- [ ] **CL-244** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify loading spinner while waiting for response

### 8.3 Error Explanation

- [ ] **CL-245** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Trigger error explanation (via context menu), verify panel opens with explanation
- [ ] **CL-246** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify explanation shows: original error, plain language, causes, fixes, docs
- [ ] **CL-247** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Trigger error explanation when AI unavailable, verify fallback text

### 8.4 Suggestions

- [ ] **CL-248** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Switch to Suggestions tab, verify suggestions listed
- [ ] **CL-249** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Generate suggestions based on context, verify new suggestions appear
- [ ] **CL-250** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Dismiss a suggestion, verify removed from list
- [ ] **CL-251** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Accept a suggestion, verify action executed or confirmation requested

### 8.5 Natural Language Commands

- [ ] **CL-252** [DEEP] [PATTERN-GROUP: PG-FORM] -- Parse natural language input "Copy all .pdf files to backup", verify parsed job config
- [ ] **CL-253** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify parsed job shows: intent, source, dest, filters, confidence
- [ ] **CL-254** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Confirm a parsed action that is destructive, verify confirmation dialog

### 8.6 AI Safety Controls

- [ ] **CL-255** [DEEP] [PATTERN-GROUP: PG-LOAD] -- View AI audit log, verify actions logged with timestamps
- [ ] **CL-256** [DEEP] [PATTERN-GROUP: PG-FORM] -- Load feature toggles, verify all toggle states
- [ ] **CL-257** [DEEP] [PATTERN-GROUP: PG-FORM] -- Disable AI master toggle, verify all AI features disabled
- [ ] **CL-258** [DEEP] [PATTERN-GROUP: PG-FORM] -- Enable only error explanations, disable suggestions, verify independent control
- [ ] **CL-259** [DEEP] [PATTERN-GROUP: PG-FORM] -- Toggle content analysis opt-in, verify state change

---

## Section 9: Desktop App -- Terminal Integration

### 9.1 Terminal Panel

- [ ] **CL-260** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Toggle terminal panel, verify panel opens at bottom of file manager
- [ ] **CL-261** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Verify first session auto-creates when panel opens with no sessions

### 9.2 Local Terminal

- [ ] **CL-262** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Create local terminal session, verify shell prompt appears
- [ ] **CL-263** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Create local session with specific cwd, verify working directory
- [ ] **CL-264** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Write to terminal session, verify output
- [ ] **CL-265** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Resize terminal, verify cols/rows update

### 9.3 Remote Terminal

- [ ] **CL-266** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Create remote terminal session (SSH) [REQUIRES: SSH server]
- [ ] **CL-267** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify remote session connected to correct host/user

### 9.4 Terminal Session Management

- [ ] **CL-268** [DEEP] [PATTERN-GROUP: PG-TABLE] -- List all terminal sessions, verify session details
- [ ] **CL-269** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Close a terminal session, verify removed from list
- [ ] **CL-270** [DEEP] [PATTERN-GROUP: PG-NAV] -- Switch between multiple terminal sessions via tabs
- [ ] **CL-271** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Set active session, verify focus moves to correct session

### 9.5 Split Terminal

- [ ] **CL-272** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Split terminal horizontally, verify two sessions side by side
- [ ] **CL-273** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Split terminal vertically, verify two sessions stacked
- [ ] **CL-274** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify split ratio is adjustable

### 9.6 Terminal Panel UI

- [ ] **CL-275** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Resize terminal panel height, verify clamped (min 150, max 600)
- [ ] **CL-276** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Panel height persists across page reloads
- [ ] **CL-277** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Get default shell, verify correct for OS
- [ ] **CL-278** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Escape a path with spaces, verify correct shell escaping

---

## Section 10: Desktop App -- Security & Encryption

### 10.1 Vault Management

- [ ] **CL-279** [SMOKE] [PATTERN-GROUP: PG-FORM] -- Create an encrypted vault (name, password), verify vault created
- [ ] **CL-280** [SMOKE] [PATTERN-GROUP: PG-FORM] -- Unlock vault with correct password, verify unlocked
- [ ] **CL-281** [DEEP] [PATTERN-GROUP: PG-FORM] -- Unlock vault with wrong password, verify error
- [ ] **CL-282** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Lock an unlocked vault, verify locked
- [ ] **CL-283** [DEEP] [PATTERN-GROUP: PG-TABLE] -- List all vaults, verify names and lock status
- [ ] **CL-284** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Check if specific vault is unlocked, verify boolean
- [ ] **CL-285** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Get unlocked vault count, verify number

### 10.2 File Encryption

- [ ] **CL-286** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Encrypt a file into vault (AES-256-GCM), verify encrypted
- [ ] **CL-287** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Decrypt a file from vault, verify original content restored
- [ ] **CL-288** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Encrypt for upload, verify encrypted blob
- [ ] **CL-289** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Decrypt from download, verify original restored

### 10.3 Vault Password

- [ ] **CL-290** [DEEP] [PATTERN-GROUP: PG-FORM] -- Change vault password (Argon2id), verify new password works
- [ ] **CL-291** [DEEP] [PATTERN-GROUP: PG-FORM] -- Change to empty password, verify rejected
- [ ] **CL-292** [DEEP] [PATTERN-GROUP: PG-FORM] -- Change to very long password (1000+ chars), verify handled

### 10.4 Transport Security

- [ ] **CL-293** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Check transport security for HTTPS URL, verify "secure"
- [ ] **CL-294** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Check transport security for HTTP URL, verify "insecure" warning
- [ ] **CL-295** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Enforce HTTPS on URL, verify HTTP upgraded to HTTPS
- [ ] **CL-296** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Get recommended security settings, verify TLS 1.3 recommended

### 10.5 Encryption Policies

- [ ] **CL-297** [DEEP] [PATTERN-GROUP: PG-FORM] -- Add encryption policy (path pattern, required algorithm), verify saved
- [ ] **CL-298** [DEEP] [PATTERN-GROUP: PG-TABLE] -- List encryption policies, verify all displayed
- [ ] **CL-299** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Remove encryption policy, verify removed
- [ ] **CL-300** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Check if path matches encryption policy, verify correct result

---

## Section 11: Desktop App -- Preview, Archive, Batch Rename, Integrity

### 11.1 Preview Pane

- [ ] **CL-301** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Select a text file, verify preview content displayed
- [ ] **CL-302** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Select an image file, verify thumbnail preview
- [ ] **CL-303** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Select an unsupported file type, verify "Preview not available" message
- [ ] **CL-304** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Get EXIF data for image, verify metadata displayed
- [ ] **CL-305** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Preview renders in sandboxed context (no script execution)

### 11.2 Archive Browser

- [ ] **CL-306** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Browse a .zip archive, verify virtual folder listing
- [ ] **CL-307** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Get archive info (format, entries, total size), verify correct
- [ ] **CL-308** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Extract archive to destination, verify files extracted
- [ ] **CL-309** [DEEP] [PATTERN-GROUP: PG-FORM] -- Create new archive from selected files, verify archive created
- [ ] **CL-310** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Browse nested archives, verify recursive listing

### 11.3 Batch Rename

- [ ] **CL-311** [SMOKE] [PATTERN-GROUP: PG-FORM] -- Open batch rename with selected files, verify rename UI
- [ ] **CL-312** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Enter rename pattern, verify live preview of new names
- [ ] **CL-313** [DEEP] [PATTERN-GROUP: PG-FORM] -- Use token `{name}`, verify original name preserved
- [ ] **CL-314** [DEEP] [PATTERN-GROUP: PG-FORM] -- Use token `{counter}`, verify sequential numbers
- [ ] **CL-315** [DEEP] [PATTERN-GROUP: PG-FORM] -- Use token `{date}`, verify date inserted
- [ ] **CL-316** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Apply batch rename, verify files renamed
- [ ] **CL-317** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Undo batch rename, verify original names restored
- [ ] **CL-318** [DEEP] [PATTERN-GROUP: PG-FORM] -- Enter pattern that produces duplicate names, verify warning

### 11.4 Integrity Tools

- [ ] **CL-319** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Compute checksum for file (MD5/SHA256), verify hash displayed
- [ ] **CL-320** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify file integrity against known checksum, verify pass/fail
- [ ] **CL-321** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Find duplicate files in directory, verify duplicates listed
- [ ] **CL-322** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Resolve duplicates (keep one, delete others), verify cleanup

### 11.5 Tags & Labels

- [ ] **CL-323** [DEEP] [PATTERN-GROUP: PG-FORM] -- Create a tag, verify tag appears in tag list
- [ ] **CL-324** [DEEP] [PATTERN-GROUP: PG-TABLE] -- List all tags, verify names displayed
- [ ] **CL-325** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Delete a tag, verify removed
- [ ] **CL-326** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Tag a file, verify tag association
- [ ] **CL-327** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Untag a file, verify tag removed
- [ ] **CL-328** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Set label on file, verify label displayed
- [ ] **CL-329** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Remove label from file, verify removed
- [ ] **CL-330** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Get file info (tags + labels + metadata), verify all shown

### 11.6 Smart Folders

- [ ] **CL-331** [DEEP] [PATTERN-GROUP: PG-FORM] -- Create smart folder with criteria, verify saved
- [ ] **CL-332** [DEEP] [PATTERN-GROUP: PG-TABLE] -- List smart folders, verify names
- [ ] **CL-333** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Delete smart folder, verify removed
- [ ] **CL-334** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Run smart folder, verify matching files returned dynamically

---

## Section 12: Desktop App -- Onboarding, Guided Flows, Simple Mode

### 12.1 Onboarding Wizard

- [ ] **CL-335** [SMOKE] [PATTERN-GROUP: PG-NAV] -- On first launch (onboardingComplete=false), verify OnboardingWizard opens
- [ ] **CL-336** [SMOKE] [PATTERN-GROUP: PG-NAV] -- Step 1 "Welcome": verify welcome message, Next button
- [ ] **CL-337** [SMOKE] [PATTERN-GROUP: PG-NAV] -- Step 2 "Choose Style": select Personal/Power/Work, verify selection
- [ ] **CL-338** [DEEP] [PATTERN-GROUP: PG-NAV] -- Choose "Power" style, verify appMode set to "advanced"
- [ ] **CL-339** [DEEP] [PATTERN-GROUP: PG-NAV] -- Choose "Personal" style, verify appMode set to "simple"
- [ ] **CL-340** [DEEP] [PATTERN-GROUP: PG-NAV] -- Step 3 "Connect Locations": verify connection setup options
- [ ] **CL-341** [DEEP] [PATTERN-GROUP: PG-NAV] -- Step 4 "First Action": verify action suggestions
- [ ] **CL-342** [DEEP] [PATTERN-GROUP: PG-NAV] -- Step 5 "Explain Compat": verify compatibility explanation
- [ ] **CL-343** [DEEP] [PATTERN-GROUP: PG-NAV] -- Step 6 "Enter Workspace": complete wizard, verify onboardingComplete=true
- [ ] **CL-344** [DEEP] [PATTERN-GROUP: PG-NAV] -- After completion, wizard does not show on next launch
- [ ] **CL-345** [DEEP] [PATTERN-GROUP: PG-NAV] -- Re-open onboarding via Help > Getting Started, verify wizard reopens

### 12.2 Simple Mode Wrapper

- [ ] **CL-346** [SMOKE] [PATTERN-GROUP: PG-NAV] -- In Simple mode, verify simplified UI with fewer controls
- [ ] **CL-347** [SMOKE] [PATTERN-GROUP: PG-NAV] -- Switch to Advanced mode, verify full UI with all features
- [ ] **CL-348** [DEEP] [PATTERN-GROUP: PG-NAV] -- Verify mode badge in toolbar shows current mode
- [ ] **CL-349** [DEEP] [PATTERN-GROUP: PG-NAV] -- Verify Simple mode hides advanced panels (sync, peer, etc.)
- [ ] **CL-350** [DEEP] [PATTERN-GROUP: PG-NAV] -- Verify mode persists across app restart

### 12.3 Guided Flows

- [ ] **CL-351** [SMOKE] [PATTERN-GROUP: PG-NAV] -- Open guided flow router, verify flow selection
- [ ] **CL-352** [DEEP] [PATTERN-GROUP: PG-NAV] -- Start "First Transfer" flow, verify step-by-step wizard
- [ ] **CL-353** [DEEP] [PATTERN-GROUP: PG-NAV] -- Verify wizard shell renders with progress indicator, Back/Next/Cancel
- [ ] **CL-354** [DEEP] [PATTERN-GROUP: PG-NAV] -- Cancel a guided flow mid-way, verify return to main UI
- [ ] **CL-355** [DEEP] [PATTERN-GROUP: PG-NAV] -- Complete a guided flow, verify success state
- [ ] **CL-356** [DEEP] [PATTERN-GROUP: PG-NAV] -- Verify operation flows (batch rename, archive, etc.)
- [ ] **CL-357** [DEEP] [PATTERN-GROUP: PG-NAV] -- Verify connection flows (SFTP setup, cloud connect, etc.)
- [ ] **CL-358** [DEEP] [PATTERN-GROUP: PG-NAV] -- Verify cloud connect flows (Google Drive, Dropbox, OneDrive, S3, B2)
- [ ] **CL-359** [DEEP] [PATTERN-GROUP: PG-NAV] -- Verify utility flows (checksum, duplicates, etc.)

### 12.4 Migration Wizard

- [ ] **CL-360** [DEEP] [PATTERN-GROUP: PG-NAV] -- Open Migration Wizard, verify 3 migration types available
- [ ] **CL-361** [DEEP] [PATTERN-GROUP: PG-NAV] -- Start computer migration wizard, verify step-by-step process
- [ ] **CL-362** [DEEP] [PATTERN-GROUP: PG-NAV] -- Start drive migration wizard, verify drive selection
- [ ] **CL-363** [DEEP] [PATTERN-GROUP: PG-NAV] -- Cancel migration wizard, verify safe exit

---

## Section 13: Desktop App -- Theme, i18n, Accessibility

### 13.1 Theme Switching

- [ ] **CL-364** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Open ThemeSwitcher, verify options: Light, Dark, System, High Contrast
- [ ] **CL-365** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Switch to Dark theme, verify `data-theme="dark"` on `<html>`
- [ ] **CL-366** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Switch to Light theme, verify `data-theme="light"` on `<html>`
- [ ] **CL-367** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Switch to System theme, verify follows OS preference
- [ ] **CL-368** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Switch to High Contrast, verify `data-theme="high-contrast"` on `<html>`
- [ ] **CL-369** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Change OS preference while on System theme, verify theme auto-switches
- [ ] **CL-370** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Theme persists across app restart

### 13.2 Internationalization

- [ ] **CL-371** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Open LanguageSelector, verify 8 locales available (en, es, fr, de, pt-BR, ja, zh-CN, ko)
- [ ] **CL-372** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Switch to Spanish (es), verify UI text changes
- [ ] **CL-373** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Switch to Japanese (ja), verify CJK text renders correctly
- [ ] **CL-374** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Switch to Korean (ko), verify Korean text renders
- [ ] **CL-375** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Language preference persists across restart
- [ ] **CL-376** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify RTL layout for Arabic locale (if supported)

### 13.3 Accessibility

- [ ] **CL-377** [SMOKE] [PATTERN-GROUP: PG-KBD] -- Verify `role="application"` on root div
- [ ] **CL-378** [SMOKE] [PATTERN-GROUP: PG-KBD] -- Verify `role="toolbar"` on main toolbar with `aria-label`
- [ ] **CL-379** [SMOKE] [PATTERN-GROUP: PG-KBD] -- Verify `role="navigation"` on sidebar with `aria-label`
- [ ] **CL-380** [DEEP] [PATTERN-GROUP: PG-KBD] -- Verify `role="contentinfo"` on status bar
- [ ] **CL-381** [DEEP] [PATTERN-GROUP: PG-KBD] -- Tab through all interactive elements, verify logical tab order
- [ ] **CL-382** [DEEP] [PATTERN-GROUP: PG-KBD] -- Verify all buttons have `aria-label` attributes
- [ ] **CL-383** [DEEP] [PATTERN-GROUP: PG-KBD] -- Verify all icons have `aria-hidden="true"`
- [ ] **CL-384** [DEEP] [PATTERN-GROUP: PG-KBD] -- Screen reader: verify toolbar announced correctly
- [ ] **CL-385** [DEEP] [PATTERN-GROUP: PG-KBD] -- Screen reader: verify file list items announced with name, type, size
- [ ] **CL-386** [DEEP] [PATTERN-GROUP: PG-KBD] -- High contrast mode: verify minimum 4.5:1 contrast ratio on all text
- [ ] **CL-387** [DEEP] [PATTERN-GROUP: PG-KBD] -- Verify focus-visible outlines on all interactive elements

### 13.4 Compatibility Notifications

- [ ] **CL-388** [DEEP] [PATTERN-GROUP: PG-TOAST] -- Trigger Tier 1 compat notification (critical), verify prominent warning
- [ ] **CL-389** [DEEP] [PATTERN-GROUP: PG-TOAST] -- Trigger Tier 2 compat notification (warning), verify subtle badge
- [ ] **CL-390** [DEEP] [PATTERN-GROUP: PG-TOAST] -- Trigger Tier 3 compat notification (info), verify compat badge on file

### 13.5 Activity Feed

- [ ] **CL-391** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Perform file operations, verify activity entries appear
- [ ] **CL-392** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Verify activity feed capped at 100 entries (MAX_ACTIVITY_ENTRIES)
- [ ] **CL-393** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Clear activity feed, verify all entries removed
- [ ] **CL-394** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify each activity entry shows: type, summary, paths, timestamp, undoable flag

### 13.6 Structured Errors

- [ ] **CL-395** [DEEP] [PATTERN-GROUP: PG-TOAST] -- Trigger error, verify structured error with: what, why, app action, user action
- [ ] **CL-396** [DEEP] [PATTERN-GROUP: PG-TOAST] -- Dismiss a structured error, verify dismissed state
- [ ] **CL-397** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Clear all errors, verify all removed
- [ ] **CL-398** [DEEP] [PATTERN-GROUP: PG-TOAST] -- Verify errors capped at 50 entries (MAX_ERRORS)

---

## Section 14: Admin Console -- Login & Dashboard

### 14.1 Dashboard (`{{BACKEND_URL}}/dashboard`)

- [ ] **CL-399** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Navigate to dashboard, verify 4 stat cards: Active Users, Pending Approvals, Online Devices, Active Policies
- [ ] **CL-400** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Verify Pending Approvals card with list of pending items
- [ ] **CL-401** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Verify Recent Activity card with audit entries
- [ ] **CL-402** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify Device Health card with device status indicators (online/degraded/offline)
- [ ] **CL-403** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify Active Policies card with enforcement mode badges
- [ ] **CL-404** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify trend indicator on Active Users stat card
- [ ] **CL-405** [DEEP] [PATTERN-GROUP: PG-NAV] -- Click "View all" on Pending Approvals, verify navigation to approvals page
- [ ] **CL-406** [DEEP] [PATTERN-GROUP: PG-NAV] -- Click "View all" on Recent Activity, verify navigation to audit page
- [ ] **CL-407** [DEEP] [PATTERN-GROUP: PG-NAV] -- Click "View fleet" on Device Health, verify navigation to devices page

---

## Section 15: Admin Console -- Users & RBAC

### 15.1 Users Page (`{{BACKEND_URL}}/users`)

- [ ] **CL-408** [SMOKE] [PATTERN-GROUP: PG-TABLE] -- Navigate to Users page, verify user table renders with columns: User, Role, Status, Last Login, Created, Actions
- [ ] **CL-409** [SMOKE] [PATTERN-GROUP: PG-TABLE] -- Verify 3 stat cards: Total Users, Active Users, SSO Users
- [ ] **CL-410** [SMOKE] [PATTERN-GROUP: PG-TABLE] -- Verify 3 tabs: Users, Roles, SSO Configuration
- [ ] **CL-411** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Search users by name, verify filtered results
- [ ] **CL-412** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Search users by email, verify filtered results
- [ ] **CL-413** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Filter by role, verify only matching role shown
- [ ] **CL-414** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Combine search + role filter, verify both applied

### 15.2 Invite User Dialog

- [ ] **CL-415** [SMOKE] [PATTERN-GROUP: PG-MODAL] -- Click "Invite User", verify dialog opens
- [ ] **CL-416** [SMOKE] [PATTERN-GROUP: PG-FORM] -- Fill email + role, click "Send Invitation", verify user added to table
- [ ] **CL-417** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with empty email, verify prevented
- [ ] **CL-418** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with invalid email format, verify error
- [ ] **CL-419** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with XSS email, verify sanitized
- [ ] **CL-420** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with SQL injection email, verify no SQL error
- [ ] **CL-421** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with extremely long email (500+ chars), verify handled
- [ ] **CL-422** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with unicode email, verify handled
- [ ] **CL-423** [DEEP] [PATTERN-GROUP: PG-MODAL] -- Click Cancel, verify dialog closes without changes

### 15.3 User Actions

- [ ] **CL-424** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Click role change icon, verify role dialog opens
- [ ] **CL-425** [DEEP] [PATTERN-GROUP: PG-FORM] -- Change user role, verify updated in table
- [ ] **CL-426** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Deactivate a user, verify status badge changes to "Inactive"
- [ ] **CL-427** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Reactivate a deactivated user, verify status badge changes to "Active"
- [ ] **CL-428** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Remove a user, verify removed from table

### 15.4 CSV Import/Export

- [ ] **CL-429** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Click "Import CSV", select valid CSV file, verify users imported
- [ ] **CL-430** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Import malformed CSV, verify handled gracefully
- [ ] **CL-431** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Click "Export CSV", verify CSV file downloads with correct headers

### 15.5 Roles Tab

- [ ] **CL-432** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Switch to Roles tab, verify all 7 roles listed with permissions
- [ ] **CL-433** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify user count badge per role
- [ ] **CL-434** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify "user" role shows "No admin permissions (standard user)"

### 15.6 SSO Configuration Tab

- [ ] **CL-435** [DEEP] [PATTERN-GROUP: PG-FORM] -- Switch to SSO tab, verify SAML 2.0 configuration form
- [ ] **CL-436** [DEEP] [PATTERN-GROUP: PG-FORM] -- Verify fields: Entity ID, SSO URL, X.509 Certificate, Enable checkbox
- [ ] **CL-437** [DEEP] [PATTERN-GROUP: PG-FORM] -- Verify SCIM Provisioning section with endpoint and token
- [ ] **CL-438** [DEEP] [PATTERN-GROUP: PG-FORM] -- Click "Save SAML Configuration", verify form submits
- [ ] **CL-439** [DEEP] [PATTERN-GROUP: PG-FORM] -- Click "Regenerate SCIM Token", verify token changes

---

## Section 16: Admin Console -- Devices

### 16.1 Devices Page (`{{BACKEND_URL}}/devices`)

- [ ] **CL-440** [SMOKE] [PATTERN-GROUP: PG-TABLE] -- Navigate to Devices, verify fleet table with columns: Device, User, Version, OS, Status, Compliance, Last Seen, Actions
- [ ] **CL-441** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Verify 4 stat cards: Online, Degraded, Offline, Non-Compliant
- [ ] **CL-442** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Search devices by name, verify filtering
- [ ] **CL-443** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Search devices by user, verify filtering
- [ ] **CL-444** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Filter by status (online/degraded/offline/error), verify filtering

### 16.2 Device Detail View

- [ ] **CL-445** [DEEP] [PATTERN-GROUP: PG-NAV] -- Click "Details" on a device, verify detail view opens
- [ ] **CL-446** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify device info: name, user, OS, version, status badge
- [ ] **CL-447** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify resource stats: CPU Usage, Memory Usage, Disk Usage
- [ ] **CL-448** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify policy compliance section (compliant or violations list)
- [ ] **CL-449** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify failure patterns section (if any)
- [ ] **CL-450** [DEEP] [PATTERN-GROUP: PG-NAV] -- Click "Back to Fleet View", verify return to fleet table

---

## Section 17: Admin Console -- Policy Engine

### 17.1 Policies Page (`{{BACKEND_URL}}/policies`)

- [ ] **CL-451** [SMOKE] [PATTERN-GROUP: PG-TABLE] -- Navigate to Policies, verify 11 domain overview cards
- [ ] **CL-452** [SMOKE] [PATTERN-GROUP: PG-TABLE] -- Verify policies table with columns: Policy, Domain, Assigned To, Mode, Updated, Status, Actions
- [ ] **CL-453** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Click a domain card, verify table filters to that domain
- [ ] **CL-454** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Search policies by name, verify filtering
- [ ] **CL-455** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Filter by domain dropdown, verify filtering

### 17.2 Create Policy Dialog

- [ ] **CL-456** [SMOKE] [PATTERN-GROUP: PG-MODAL] -- Click "Create Policy", verify dialog opens with form
- [ ] **CL-457** [SMOKE] [PATTERN-GROUP: PG-FORM] -- Fill all fields (domain, name, description, target, value, mode), submit, verify policy created
- [ ] **CL-458** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with empty name, verify prevented
- [ ] **CL-459** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with XSS in name, verify sanitized
- [ ] **CL-460** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with SQL injection in description, verify safe
- [ ] **CL-461** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with unicode characters in name, verify handled
- [ ] **CL-462** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with very long name (500+ chars), verify handled
- [ ] **CL-463** [DEEP] [PATTERN-GROUP: PG-FORM] -- Verify domain description updates when selecting different domains
- [ ] **CL-464** [DEEP] [PATTERN-GROUP: PG-FORM] -- Verify assignment target options: org, connection_type, role, user
- [ ] **CL-465** [DEEP] [PATTERN-GROUP: PG-FORM] -- Verify enforcement modes: enforce, warn, audit_only, disabled

### 17.3 Policy Actions

- [ ] **CL-466** [DEEP] [PATTERN-GROUP: PG-NAV] -- Click "Edit" on a policy, verify detail dialog opens
- [ ] **CL-467** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Toggle policy active/disabled, verify status badge changes
- [ ] **CL-468** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Delete a policy, verify removed from table
- [ ] **CL-469** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Click "Propagate" on a policy, verify spinning indicator then completion
- [ ] **CL-470** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Create conflicting policies, verify conflict detection (console warning)

---

## Section 18: Admin Console -- Approval Workflows

### 18.1 Approvals Page (`{{BACKEND_URL}}/approvals`)

- [ ] **CL-471** [SMOKE] [PATTERN-GROUP: PG-TABLE] -- Navigate to Approvals, verify 5 stat cards: Pending, Approved, Denied, Expired, Avg Response
- [ ] **CL-472** [SMOKE] [PATTERN-GROUP: PG-TABLE] -- Verify 5 tabs: Pending, Approved, Denied, Expired, All
- [ ] **CL-473** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Switch between tabs, verify correct approvals displayed
- [ ] **CL-474** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Search approvals by operation/requester/reason, verify filtering
- [ ] **CL-475** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Filter by trigger type, verify filtering

### 18.2 Approval Cards

- [ ] **CL-476** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify pending approval shows: state badge, trigger badge, time remaining, operation, source/dest, file count/size, requester, reason
- [ ] **CL-477** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify completed approval shows reviewer name and comment

### 18.3 Review Dialog

- [ ] **CL-478** [SMOKE] [PATTERN-GROUP: PG-MODAL] -- Click "Approve" on pending approval, verify review dialog opens
- [ ] **CL-479** [SMOKE] [PATTERN-GROUP: PG-CONFIRM] -- Add review comment and approve, verify state changes to "Approved"
- [ ] **CL-480** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Deny an approval with comment, verify state changes to "Denied"
- [ ] **CL-481** [DEEP] [PATTERN-GROUP: PG-MODAL] -- Click Cancel in review dialog, verify no changes made
- [ ] **CL-482** [DEEP] [PATTERN-GROUP: PG-LOAD] -- View details of already-reviewed approval, verify read-only view

### 18.4 Auto-Expiry

- [ ] **CL-483** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify time remaining countdown on pending approvals
- [ ] **CL-484** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify auto-expiry check runs periodically (every 60s)

---

## Section 19: Admin Console -- Audit Explorer

### 19.1 Audit Page (`{{BACKEND_URL}}/audit`)

- [ ] **CL-485** [SMOKE] [PATTERN-GROUP: PG-TABLE] -- Navigate to Audit, verify audit table with columns: Severity icon, Event, User, Description, IP, Time, Actions
- [ ] **CL-486** [SMOKE] [PATTERN-GROUP: PG-TABLE] -- Verify 2 tabs: Audit Explorer, Event Categories
- [ ] **CL-487** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Search audit entries by description/user/event type, verify filtering
- [ ] **CL-488** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Filter by severity (info/warning/error/critical), verify filtering
- [ ] **CL-489** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Filter by category, verify filtering
- [ ] **CL-490** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Filter by date range (from/to), verify filtering
- [ ] **CL-491** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Combine all filters simultaneously, verify correct results

### 19.2 Audit Entry Details

- [ ] **CL-492** [DEEP] [PATTERN-GROUP: PG-MODAL] -- Click "Details" on audit entry, verify detail dialog opens
- [ ] **CL-493** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify entry shows: event type, severity, timestamp, IP, user, resource, description, metadata JSON, integrity hash, previous hash

### 19.3 Integrity Verification

- [ ] **CL-494** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Click "Verify Integrity", verify chain verification result displayed
- [ ] **CL-495** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Dismiss verification result, verify banner removed

### 19.4 Export

- [ ] **CL-496** [DEEP] [PATTERN-GROUP: PG-MODAL] -- Click "Export", verify export dialog with 3 format options
- [ ] **CL-497** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Export as CSV, verify file downloads
- [ ] **CL-498** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Export as JSON, verify file downloads
- [ ] **CL-499** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Export as Syslog, verify file downloads

### 19.5 SIEM Configuration

- [ ] **CL-500** [DEEP] [PATTERN-GROUP: PG-MODAL] -- Click "SIEM Config", verify SIEM dialog opens
- [ ] **CL-501** [DEEP] [PATTERN-GROUP: PG-FORM] -- Fill SIEM config: type (webhook/syslog), endpoint, format (JSON/CEF/LEEF), auth token
- [ ] **CL-502** [DEEP] [PATTERN-GROUP: PG-FORM] -- Toggle TLS enabled, verify checkbox state
- [ ] **CL-503** [DEEP] [PATTERN-GROUP: PG-FORM] -- Toggle SIEM streaming enabled, verify checkbox state
- [ ] **CL-504** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with empty endpoint, verify validation
- [ ] **CL-505** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with invalid URL, verify validation
- [ ] **CL-506** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with XSS in endpoint, verify sanitized
- [ ] **CL-507** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with SQL injection in endpoint, verify safe
- [ ] **CL-508** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with very long endpoint (1000+ chars), verify handled

### 19.6 Event Categories Tab

- [ ] **CL-509** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Switch to Event Categories tab, verify all categories listed
- [ ] **CL-510** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify event type badges show count per type

---

## Section 20: Admin Console -- Connectors

### 20.1 Connectors Page (`{{BACKEND_URL}}/connectors`)

- [ ] **CL-511** [SMOKE] [PATTERN-GROUP: PG-TABLE] -- Navigate to Connectors, verify connector cards listed
- [ ] **CL-512** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify each connector shows: name, type, status badge, last tested date
- [ ] **CL-513** [DEEP] [PATTERN-GROUP: PG-EMPTY] -- Delete all connectors, verify empty state with "Add Connector" CTA

### 20.2 Add Connector Dialog

- [ ] **CL-514** [SMOKE] [PATTERN-GROUP: PG-MODAL] -- Click "Add Connector", verify dialog opens
- [ ] **CL-515** [SMOKE] [PATTERN-GROUP: PG-FORM] -- Fill name + type, submit, verify connector added
- [ ] **CL-516** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with empty name, verify prevented
- [ ] **CL-517** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with XSS in name, verify sanitized
- [ ] **CL-518** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with SQL injection in name, verify safe
- [ ] **CL-519** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with unicode name, verify handled
- [ ] **CL-520** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with very long name (500+ chars), verify handled
- [ ] **CL-521** [DEEP] [PATTERN-GROUP: PG-FORM] -- Verify all 8 connector types available: S3, Azure Blob, GCS, SFTP, FTP, WebDAV, SMB, Local

### 20.3 Connector Actions

- [ ] **CL-522** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Click "Test" on a connector, verify loading spinner then status update
- [ ] **CL-523** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Click "Remove" on a connector, verify removed from list
- [ ] **CL-524** [DEEP] [PATTERN-GROUP: PG-NAV] -- Click "Settings" on a connector, verify settings interaction

---

## Section 21: Admin Console -- AI Governance

### 21.1 AI Governance Page (`{{BACKEND_URL}}/ai-governance`)

- [ ] **CL-525** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Navigate to AI Governance, verify 3 stat cards: AI Features count, Data Residency, Excluded Paths
- [ ] **CL-526** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Verify Global AI Settings card with Data Residency dropdown and Excluded Paths input

### 21.2 Global Settings

- [ ] **CL-527** [DEEP] [PATTERN-GROUP: PG-FORM] -- Change Data Residency to "Cloud", verify selection
- [ ] **CL-528** [DEEP] [PATTERN-GROUP: PG-FORM] -- Change Data Residency to "Hybrid", verify selection
- [ ] **CL-529** [DEEP] [PATTERN-GROUP: PG-FORM] -- Change Data Residency to "Local Only", verify selection
- [ ] **CL-530** [DEEP] [PATTERN-GROUP: PG-FORM] -- Edit excluded paths, verify comma-separated input
- [ ] **CL-531** [DEEP] [PATTERN-GROUP: PG-FORM] -- Click "Save Settings", verify form submits

### 21.3 Feature Controls

- [ ] **CL-532** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Verify 4 AI features listed: File Classification, Content Analysis, Naming Suggestions, Anomaly Detection
- [ ] **CL-533** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Toggle File Classification off, verify status badge changes to "Disabled"
- [ ] **CL-534** [DEEP] [PATTERN-GROUP: PG-CONFIRM] -- Toggle Content Analysis on, verify status badge changes to "Enabled"
- [ ] **CL-535** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify data residency badge per feature

---

## Section 22: Admin Console -- Billing

### 22.1 Billing Page (`{{BACKEND_URL}}/billing`)

- [ ] **CL-536** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Navigate to Billing, verify 3 stat cards: Current Plan, Active Users, Storage Used
- [ ] **CL-537** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Verify 3 plan cards: Starter (Free), Business ($29/user/mo), Enterprise (Custom)
- [ ] **CL-538** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify current plan has "Current Plan" badge and disabled button
- [ ] **CL-539** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify Starter plan has "Upgrade" button
- [ ] **CL-540** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify Enterprise plan has "Contact Sales" button
- [ ] **CL-541** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify feature list for each plan

### 22.2 Billing History

- [ ] **CL-542** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Verify billing history table with: invoice number, date, amount, status
- [ ] **CL-543** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify "Paid" badge on each invoice
- [ ] **CL-544** [DEEP] [PATTERN-GROUP: PG-NAV] -- Click download icon on invoice, verify download action

---

## Section 23: Admin Console -- Shared Workspaces

### 23.1 Workspaces Page (`{{BACKEND_URL}}/workspaces`)

- [ ] **CL-545** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Navigate to Workspaces, verify 3 stat cards: Shared Connections, Sync Templates, Team Members
- [ ] **CL-546** [SMOKE] [PATTERN-GROUP: PG-TABLE] -- Verify 4 tabs: Shared Connections, Sync Templates, Automation Templates, Members

### 23.2 Shared Connections Tab

- [ ] **CL-547** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify shared connection cards with: name, type, owner, active status, propagation status
- [ ] **CL-548** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify shared members list on each connection card
- [ ] **CL-549** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Click "Propagate" on a connection, verify spinner then completion

### 23.3 Sync Templates Tab

- [ ] **CL-550** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Switch to Sync Templates, verify template cards with: name, mode, source/dest, schedule
- [ ] **CL-551** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify "Activate" button and "Propagate" button on each template
- [ ] **CL-552** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify activated by count and shared with count

### 23.4 Automation Templates Tab

- [ ] **CL-553** [DEEP] [PATTERN-GROUP: PG-EMPTY] -- Switch to Automation Templates, verify empty state with "Create Template" CTA

### 23.5 Members Tab

- [ ] **CL-554** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Switch to Members, verify members table with: Member, Access Level, Added, Added By
- [ ] **CL-555** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify access level badges (admin/write/read)

### 23.6 Share Resource Dialog

- [ ] **CL-556** [SMOKE] [PATTERN-GROUP: PG-MODAL] -- Click "Share Resource", verify dialog opens
- [ ] **CL-557** [DEEP] [PATTERN-GROUP: PG-FORM] -- Verify form fields: Resource Type, User Email, Access Level
- [ ] **CL-558** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with empty email, verify prevented
- [ ] **CL-559** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with XSS email, verify sanitized
- [ ] **CL-560** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with SQL injection email, verify safe
- [ ] **CL-561** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with unicode email, verify handled
- [ ] **CL-562** [DEEP] [PATTERN-GROUP: PG-FORM] -- Submit with very long email (500+ chars), verify handled
- [ ] **CL-563** [DEEP] [PATTERN-GROUP: PG-MODAL] -- Click Cancel, verify dialog closes

---

## Section 24: Responsive & Viewport Testing

### 24.1 Desktop App Window Sizes

- [ ] **CL-564** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Window at 1280x800 (default), verify full layout renders correctly
- [ ] **CL-565** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Window at 800x600 (minimum), verify layout does not break, all controls accessible
- [ ] **CL-566** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Window at 1920x1080 (HD), verify layout scales appropriately
- [ ] **CL-567** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Window at 2560x1440 (2K), verify no scaling issues
- [ ] **CL-568** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Window below minimum (e.g. 600x400), verify resize is prevented (minWidth: 800, minHeight: 600)
- [ ] **CL-569** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Resize window dynamically, verify layout reflows without broken elements

### 24.2 Admin Console Viewports

- [ ] **CL-570** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Admin at 1440px wide, verify full desktop layout
- [ ] **CL-571** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Admin at 1024px (tablet landscape), verify responsive columns
- [ ] **CL-572** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Admin at 768px (tablet portrait), verify sidebar collapses, tables scroll horizontally
- [ ] **CL-573** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Admin at 640px (mobile landscape), verify mobile layout
- [ ] **CL-574** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Admin at 375px (mobile portrait), verify single-column layout, all content accessible
- [ ] **CL-575** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Verify `hidden md:table-cell` columns hide below md breakpoint
- [ ] **CL-576** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Verify `hidden lg:table-cell` columns hide below lg breakpoint
- [ ] **CL-577** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify stat card grid: 1 col on mobile, 2 col on sm, 4 col on lg

---

## Section 25: Performance & Stress Testing

### 25.1 File List Performance

- [ ] **CL-578** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Load directory with 10,000 files (TanStack Virtual), verify smooth scrolling
- [ ] **CL-579** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify only visible rows rendered (virtual scrolling DOM check)
- [ ] **CL-580** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Load directory with 100,000 files, verify no crash, acceptable scroll performance
- [ ] **CL-581** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Filter 10K file list, verify filter completes in < 100ms

### 25.2 State Management

- [ ] **CL-582** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Open 20 tabs across both panes, verify no memory leak or slowdown
- [ ] **CL-583** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Generate 100 activity feed entries, verify feed renders and oldest entries evicted

### 25.3 Workspace Restore

- [ ] **CL-584** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Save complex workspace state (tabs, favorites, recent, selections), restart app, verify restore in < 500ms

### 25.4 Transfer Performance

- [ ] **CL-585** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Enqueue 100 transfers simultaneously, verify queue processes without crash
- [ ] **CL-586** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Transfer large file (1 GB+), verify progress updates and completion

### 25.5 Admin Console Performance

- [ ] **CL-587** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Dashboard with 100+ audit entries, verify renders within 2 seconds
- [ ] **CL-588** [DEEP] [PATTERN-GROUP: PG-TABLE] -- Users table with 500 users, verify search/filter responsive

---

## Section 26: Cross-Platform & Build Verification

### 26.1 macOS

- [ ] **CL-589** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Build and run on macOS (minimum 10.15), verify app launches
- [ ] **CL-590** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify macOS menu bar integration
- [ ] **CL-591** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify Cmd keyboard shortcuts (not Ctrl)
- [ ] **CL-592** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify .dmg / .app bundle builds

### 26.2 Windows

- [ ] **CL-593** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Build and run on Windows 10+, verify app launches
- [ ] **CL-594** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify Ctrl keyboard shortcuts (not Cmd)
- [ ] **CL-595** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify .msi (WiX) and .exe (NSIS) installers build
- [ ] **CL-596** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify NSIS language selector shows 8 languages
- [ ] **CL-597** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify WebView2 bootstrapper installation

### 26.3 Linux

- [ ] **CL-598** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Build and run on Ubuntu 22.04+ (deb), verify app launches
- [ ] **CL-599** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Build .rpm package, verify installs on Fedora
- [ ] **CL-600** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Build AppImage, verify runs without installation
- [ ] **CL-601** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify libwebkit2gtk, libgtk-3, libayatana-appindicator3 dependencies

### 26.4 File System Compatibility

- [ ] **CL-602** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Create files with Windows-reserved names (CON, PRN, NUL) on macOS/Linux, verify compat warning
- [ ] **CL-603** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Handle paths with backslashes on Unix, verify normalized
- [ ] **CL-604** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Handle paths > 260 chars (Windows MAX_PATH), verify warning or long path support
- [ ] **CL-605** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Handle filenames with unicode (emoji, CJK, Arabic, Devanagari), verify correct display

---

## Section 27: Persistence & Crash Recovery

### 27.1 State Persistence

- [ ] **CL-606** [SMOKE] [PATTERN-GROUP: PG-LOAD] -- Close and reopen app, verify workspace state restored (tabs, panes, favorites, recent)
- [ ] **CL-607** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify localStorage keys: `ufop-ui-state`, `ufop-file-manager-state`, `ufop-ai-state`, `ufop-terminal-state`
- [ ] **CL-608** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Clear localStorage, restart, verify clean defaults (onboarding shows, simple mode)

### 27.2 Transfer Queue Persistence

- [ ] **CL-609** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Enqueue transfers, close app, reopen, verify queued transfers restored
- [ ] **CL-610** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Active transfer during crash, verify re-queued on next launch (Active -> Queued)

### 27.3 Journal Recovery

- [ ] **CL-611** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify three-layer transfer journal initialized on startup
- [ ] **CL-612** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Simulate crash mid-transfer, verify journal recovery on restart

### 27.4 Database Fallback

- [ ] **CL-613** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Corrupt SQLite database file, verify app falls back to in-memory database
- [ ] **CL-614** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify fallback logs warning "Starting with in-memory database as fallback"

### 27.5 Shutdown Persistence

- [ ] **CL-615** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify `on_window_event(Destroyed)` persists transfer queue
- [ ] **CL-616** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Close window while transfers active, reopen, verify transfers preserved

---

## Section 28: Auto-Update

### 28.1 Update Mechanism

- [ ] **CL-617** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify auto-update check runs periodically (every 4 hours)
- [ ] **CL-618** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify updater plugin configuration: active=true, dialog=true
- [ ] **CL-619** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify updater endpoints configured (releases.ufop.app + GitHub releases)
- [ ] **CL-620** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify public key placeholder requires replacement before production
- [ ] **CL-621** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify CSP allows `connect-src` to update endpoints

---

## Appendix A: Admin Console Navigation Sidebar

All admin routes must be accessible from the sidebar navigation:

- [ ] **CL-622** [SMOKE] [PATTERN-GROUP: PG-NAV] -- Verify sidebar links: Dashboard, Users, Devices, Policies, Approvals, Audit, Connectors, AI Governance, Billing, Workspaces
- [ ] **CL-623** [DEEP] [PATTERN-GROUP: PG-NAV] -- Verify active route highlight in sidebar
- [ ] **CL-624** [DEEP] [PATTERN-GROUP: PG-NAV] -- Verify sidebar responsive collapse on small screens
- [ ] **CL-625** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify header component renders on all admin pages
- [ ] **CL-626** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify PageHeader component shows title + description on every page

---

## Appendix B: Tauri Plugin Verification

- [ ] **CL-627** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify `tauri-plugin-shell` loaded (shell command execution)
- [ ] **CL-628** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify `tauri-plugin-dialog` loaded (native file dialogs)
- [ ] **CL-629** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify `tauri-plugin-fs` loaded (filesystem access)
- [ ] **CL-630** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify `tauri-plugin-os` loaded (OS information)
- [ ] **CL-631** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify `tauri-plugin-updater` loaded (auto-update)

---

## Appendix C: Content Security Policy

- [ ] **CL-632** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify CSP header: `default-src 'self'`
- [ ] **CL-633** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify CSP header: `img-src 'self' asset: https://asset.localhost`
- [ ] **CL-634** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify CSP header: `style-src 'self' 'unsafe-inline'`
- [ ] **CL-635** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Verify CSP header: `connect-src 'self' https://releases.ufop.app https://api.github.com`
- [ ] **CL-636** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Attempt to load external script, verify blocked by CSP
- [ ] **CL-637** [DEEP] [PATTERN-GROUP: PG-LOAD] -- Attempt inline script injection, verify blocked by CSP

---

## Appendix D: Zustand Store Coverage Matrix

| Store | Persisted Keys | Test |
|-------|---------------|------|
| `ufop-ui-state` | theme, sidebarOpen, sidebarWidth, appMode, viewMode, statusBarVisible, previewPanelOpen, previewPanelWidth, onboardingComplete, userStyle, activityFeed | CL-607 |
| `ufop-file-manager-state` | panes, singlePaneMode, paneSplitPercent, favorites, recentLocations, savedSearches | CL-607 |
| `ufop-ai-state` | panelOpen, activeTab, featureToggles | CL-607 |
| `ufop-terminal-state` | panelOpen, panelHeight | CL-607 |

---

## Appendix E: IPC Command Coverage Matrix

All 164 Tauri IPC commands must be reachable through UI or automated test:

| Module | Commands | Count |
|--------|----------|-------|
| system_commands | greet, get_app_version, get_platform_info | 3 |
| fs_commands | list_directory | 1 |
| state_commands | save_workspace_state, load_workspace_state, get_config, set_config, reset_database | 5 |
| transfer_commands | enqueue_transfer, pause_transfer, resume_transfer, cancel_transfer, list_transfers, get_transfer, set_transfer_priority, reorder_transfer, set_global_throttle, set_connection_throttle, get_throttle_config, update_transfer_progress, complete_transfer, fail_transfer, retry_transfer, retry_all_failed, list_failed_transfers, set_retry_policy, check_conflict, resolve_conflict, set_conflict_policy, set_transfer_conflict_policy, verify_transfer, compute_checksum, set_verify_checksums, set_checksum_algorithm, search_transfer_history, export_transfer_history, cleanup_transfer_history, set_verify_tier, get_verify_tier, get_transfer_config, set_transfer_config | 33 |
| connection_commands | save_connection, list_connections, get_connection, delete_connection, test_connection, search_connections, create_connection_group, list_connection_groups, delete_connection_group, export_connections, import_connections | 11 |
| connector_commands | connector_connect, connector_disconnect, connector_list_remote, connector_is_connected, connector_list_protocols | 5 |
| drive_commands | smb_discover_shares, smb_list_shares, nfs_list_exports, detect_drives, transfer_preflight, eject_drive | 6 |
| sync_commands | create_sync_pair, delete_sync_pair, list_sync_pairs, update_sync_pair, run_sync, get_sync_health, sync_dry_run, export_sync_preview_csv, get_sync_conflicts, resolve_sync_conflict, get_quarantine_entries, get_sync_reports, rollback_sync, export_sync_report_csv, export_sync_report_json, start_sync_watcher, stop_sync_watcher, validate_sync_cron | 18 |
| peer_commands | peer_start_discovery, peer_stop_discovery, peer_is_discovering, peer_list_peers, peer_list_online, peer_get_peer, peer_set_trust, peer_save_peer, peer_remove_saved, peer_list_saved, peer_set_display_name, peer_connect_manual, peer_request_transfer, peer_list_pending_requests, peer_respond_transfer, peer_get_transfer_progress, peer_list_transfers, peer_cancel_transfer, server_transfer_preview_method, server_transfer_create, server_transfer_start, server_transfer_pause, server_transfer_cancel, server_transfer_retry, server_transfer_get, server_transfer_list, server_transfer_list_active, server_transfer_capability_matrix, server_transfer_cleanup | 29 |
| ai_commands | ai_explain_error, ai_chat, ai_get_chat_history, ai_clear_chat, ai_generate_suggestions, ai_get_suggestions, ai_dismiss_suggestion, ai_accept_suggestion, ai_parse_natural_language, ai_confirm_action, ai_check_confirmation_needed, ai_get_audit_log, ai_get_feature_toggles, ai_set_feature_toggles | 14 |
| terminal_commands | terminal_create_local, terminal_create_remote, terminal_list_sessions, terminal_close_session, terminal_write, terminal_resize, terminal_set_layout, terminal_get_layout, terminal_escape_path, terminal_get_default_shell | 10 |
| encryption_commands | create_vault, unlock_vault, lock_vault, list_vaults, is_vault_unlocked, get_unlocked_vault_count, vault_encrypt_file, vault_decrypt_file, encrypt_for_upload, decrypt_from_download, change_vault_password, check_transport_security, check_transport_security_custom, get_recommended_security, enforce_https_url, add_encryption_policy, list_encryption_policies, remove_encryption_policy, check_encryption_policy | 19 |
| file_ops_commands | copy_files, move_files, rename_file, duplicate_files, delete_files, create_directory, create_file, undo_file_operation, get_file_metadata | 9 |
| batch_rename_commands | batch_rename_preview, batch_rename_apply, batch_rename_undo | 3 |
| preview_commands | preview_file, preview_get_exif | 2 |
| archive_commands | archive_browse, archive_create, archive_extract, archive_info | 4 |
| integrity_commands | integrity_checksum, integrity_verify, integrity_find_duplicates, integrity_resolve_duplicates, integrity_create_tag, integrity_list_tags, integrity_delete_tag, integrity_tag_file, integrity_untag_file, integrity_get_file_info, integrity_set_label, integrity_remove_label, integrity_create_smart_folder, integrity_list_smart_folders, integrity_delete_smart_folder, integrity_run_smart_folder | 16 |
| **TOTAL** | | **188** |

---

## Summary

| Metric | Count |
|--------|-------|
| Total Checklist Items | 637 |
| SMOKE items | ~65 |
| DEEP items | ~572 |
| Sections | 28 + 5 Appendices |
| Desktop App items | ~395 |
| Admin Console items | ~180 |
| Responsive/Performance/Platform items | ~62 |
| Unique Pattern Groups | 10 |
| IPC Commands Covered | 188 |
| Connectors Covered | 12 (SFTP, FTP, FTPS, WebDAV, SMB, NFS, S3, GDrive, Dropbox, OneDrive, B2, Local) |

---

*Generated for Unified File Operations Platform v0.1.0*
*Tauri 2 + Rust + React/Vite (Desktop) + Next.js 15 (Admin Console)*
