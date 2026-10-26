# RcloneView parity — implementation checklist

Source: the RcloneView gap analysis (63 rows to implement). Row numbers are the analysis's; WS-G's nine items are listed by name because their row numbers were not carried into the commits.
Branch: `feat/rcloneview-parity` (integration), WS-G on `claude/determined-feynman-n2217r`. Commits name the workstream (`WS-X …`).

A box is ticked only when the engine code, its wiring (IPC / CLI / UI) and its tests are in and passing.

## WS-A — Connections and sign-in (rows 19, 23–27, 87)
- [x] OAuth sign-in for Google Drive, Dropbox, OneDrive; business scopes; B2 application key; SFTP/WebDAV/SMB profile fields; starred connections; rejected-credential handling; remote quota and folder size

## WS-B — rclone (rows 31, 32, 37, 75, 80)
- [x] rclone resolver, settings and managed install; in-app remote manager (schema forms, OAuth); long-tail backends; crypt-aware browsing; engine version in diagnostics

## WS-C — Sync (rows 42, 44, 45, 47, 49, 50, 52, 53, 55, 56, 58)
- [x] Two-way baseline, checksum/size-only compare, version pruning, resume checkpoints, one filter engine with presets and filter-from, global sync defaults, bandwidth timetable, edit/duplicate/export/import jobs

## WS-D — Folder compare (rows 38, 39, 40)
- [x] Standalone compare (local and connections, cancellable), filters and navigation, apply/delete selected differences

## WS-E — Mounts (rows 66, 67, 70)
- [x] Mount dialog and Mounts list reachable, advanced mount options, open-after-mount and tray mount controls

## WS-F — Automation and notifications (rows 60, 62–65, 76–79, 85)
- [x] Desktop notifications for transfers and sync, shared notification profiles, size threshold, next run, template search, step risk, new steps, step outputs, URL-to-remote, chat-bot control

## WS-H — Cloud locations in the panes (rows 8, 11, 12, 13, 17)
- [x] `conn://` pane locations, cross-location copy/move via the queue, drag-and-drop semantics and confirmation, editable address bar, remote folder picker, default upload/download folders

## WS-I — History, logs, server, feedback (rows 72, 73, 74, 82, 84, 93, 100)
- [x] Run history detail, log viewer, failed jobs kept over restarts, embedded server and server profiles, terminal command help, in-app feedback

## WS-J — Billing and docs (rows 95, 96, 97)
- [x] Trial and annual billing, coupons, searchable docs

## WS-G — Interface (9 items)
- [x] G1 Language selection — Settings → Appearance and onboarding; chrome, palette prompt and tray follow it; an English choice now sticks (regression test)
- [x] G2 Accent colour — presets and custom; tokens and every `blue-*` utility follow it; high contrast unchanged
- [x] G3 Startup and tray behaviour — start at login (optionally in the tray), start minimised, minimise/close to tray, tray icon; never hides the window without a tray
- [x] G4 Reset — appearance and layout, startup/tray/notifications, sync and transfer defaults, welcome tour; data and integrations kept
- [x] G5 Wizards — all 18 guided flows do the real work and are in the command palette; compat engine exposed (scan, reversible renames, restore)
- [x] G6 Parent-folder shortcut — Up button, Alt+↑ / ⌘↑ / Backspace, palette; drive, share and OneDrive roots fixed (regression test)
- [x] G7 Detection of local cloud-sync folders — Dropbox, OneDrive, Google Drive, iCloud, Box, pCloud, MEGA, Nextcloud, ownCloud, Synology, Proton; sidebar section
- [x] G8 App lock — master password at start, after idle minutes or now; IPC gate while locked (IPC-level test); calls held until unlock
- [x] G9 Multiple windows — New Window (Ctrl/⌘+N, palette, context menu, tray); own panes per window, shared settings follow; shutdown only with the last window

## Follow-ups (from the reviewers' notes; tracked separately)
- [ ] WS-Z1 — 25 backend fixes, mostly data safety and security (e.g. SMB password visible in the process list on macOS/Windows; two-way sync deleting a file edited between planning and running; compare folder-delete race; purge step following symlinked parents; OAuth tokens lost on a badly timed network error). The full list is in the reviewers' notes, not in this repository.
- [ ] WS-Z2 — 21 polish fixes (e.g. rclone forms writing empty settings; billing labels and promo-code mentions; address bar rejecting local paths with `:`; long remote file names failing to download; an interrupted cloud move returning as a copy after a restart; a gap in the log viewer's live tail). Full list as above.

## Final verification (WS-G)
- [ ] src-tauri `cargo test --lib`, `cargo clippy --all-targets -D warnings`
- [x] Web vitest, tsc, eslint on the changed files; `dump-ipc-commands.sh --verify`, `check-ipc-args.sh`, `check-appstate.sh`, `check-tauri-capabilities.sh`, `check-tauri-config.sh`
- [x] Docs (user guide sections for each item, IPC reference regenerated) and CHANGELOG entries
- [ ] Independent review of the WS-G diff, findings fixed
