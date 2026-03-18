# UFOP Release Checklist

Version 0.1.0

---

## Pre-Release Validation

### P0 Bug Verification

- [ ] No open P0 (critical/blocker) issues
- [ ] No data-loss bugs in transfer engine
- [ ] No destructive-sync bugs (files deleted unintentionally)
- [ ] No credential leakage paths
- [ ] Crash recovery works (transfer journal recovers after force-quit)

### Compatibility Corpus

- [ ] Compatibility engine passes full test corpus
- [ ] Windows reserved name detection works (CON, PRN, AUX, NUL, COM1-9, LPT1-9)
- [ ] Unicode normalization (NFC/NFD) roundtrip test passes
- [ ] Path length validation works for Windows 260-char limit
- [ ] Special character handling works across platforms

### Credential Storage

- [ ] macOS Keychain integration tested and approved
- [ ] Windows Credential Manager integration tested
- [ ] Linux Secret Service (GNOME Keyring / KWallet) integration tested
- [ ] Credentials not included in connection exports
- [ ] Credentials not logged in any output

### Transfer Engine

- [ ] Transfer enqueue/pause/resume/cancel all work
- [ ] Retry logic handles transient failures
- [ ] Bandwidth throttling works (global and per-connection)
- [ ] Checksum verification works for all three tiers
- [ ] Transfer history search and CSV export work
- [ ] Crash recovery restores interrupted transfers

### Sync Engine

- [ ] Sync pair create/run/delete all work
- [ ] Bidirectional sync handles conflicts correctly
- [ ] Filesystem watcher triggers sync on changes
- [ ] Scheduled sync (cron) triggers at correct times
- [ ] Dry run produces accurate preview
- [ ] Rollback restores pre-sync state
- [ ] Health indicators update after each run

### Connectors

- [ ] SFTP: Connect, list, transfer, disconnect
- [ ] FTP/FTPS: Connect, list, transfer (active and passive mode)
- [ ] WebDAV: Connect, list, transfer (Nextcloud, ownCloud)
- [ ] SMB: Discover shares, connect, list, transfer
- [ ] NFS: List exports, mount, list, transfer
- [ ] S3: Connect, list buckets, upload, download
- [ ] Google Drive: OAuth flow, list, upload, download
- [ ] Dropbox: OAuth flow, list, upload, download
- [ ] OneDrive: OAuth flow, list, upload, download
- [ ] B2: Connect, list, upload, download
- [ ] Local Drive: Detect, preflight, transfer, eject

### AI Features

- [ ] Chat produces relevant responses
- [ ] Error explanation provides actionable advice
- [ ] Suggestions are contextually appropriate
- [ ] Natural language parsing creates correct job configs
- [ ] Destructive action confirmation gate works
- [ ] Audit log captures all AI interactions
- [ ] Feature toggles disable features as expected

### Encryption

- [ ] Vault creation with password works
- [ ] Vault unlock/lock cycle works
- [ ] File encrypt/decrypt roundtrip produces identical output
- [ ] Encrypt-for-upload / decrypt-from-download works
- [ ] Password change works without data loss
- [ ] Transport security check flags insecure connections

---

## Installer Validation

### macOS

- [ ] DMG installer opens correctly
- [ ] App launches from /Applications
- [ ] Notarization status (if applicable)
- [ ] Minimum OS version check (10.15)
- [ ] Auto-updater works

### Windows

- [ ] NSIS installer completes without errors
- [ ] WiX MSI installer completes without errors
- [ ] Language selection works (8 languages)
- [ ] WebView2 auto-install works on clean system
- [ ] App launches from Start Menu
- [ ] Auto-updater works (passive mode)

### Linux

- [ ] .deb installs on Ubuntu/Debian
- [ ] .rpm installs on Fedora/RHEL
- [ ] .AppImage runs without installation
- [ ] Dependencies are correctly declared
- [ ] Desktop integration works (app icon, launcher)

---

## Documentation

- [ ] User guide covers all Simple mode flows
- [ ] Admin guide covers all admin console features
- [ ] CLI reference matches actual command behavior
- [ ] API reference matches OpenAPI spec
- [ ] Troubleshooting guide covers known issues
- [ ] Migration guide covers supported import paths
- [ ] Security review is current
- [ ] CHANGELOG lists all notable changes
- [ ] Release notes are written

---

## Infrastructure

### Crash Reporting

- [ ] Crash handler is functional
- [ ] Error reports are sanitized (no credentials, no PII)
- [ ] Crash reports are delivered to the reporting endpoint

### Auto-Updater

- [ ] Update endpoint is configured (`https://releases.ufop.app`)
- [ ] Fallback endpoint works (`https://github.com/ufop/unified-file-ops/releases`)
- [ ] Update public key is set (replace placeholder before production)
- [ ] Update dialog displays correctly
- [ ] Rollback mechanism exists if update fails

### Rollback Plan

- [ ] Previous version installers are archived
- [ ] Database downgrade path documented (or forward-compatible)
- [ ] Rollback instructions written for support team

---

## Final Smoke Test

### Quick Verification (15 minutes)

1. [ ] Fresh install on each platform (macOS, Windows, Linux)
2. [ ] Onboarding wizard completes
3. [ ] Browse local files
4. [ ] Create a folder and a file
5. [ ] Copy a file
6. [ ] Connect to one cloud service (OneDrive, GDrive, or Dropbox)
7. [ ] Transfer a file to the cloud service
8. [ ] Create a sync pair and run once
9. [ ] Switch between Simple and Advanced mode
10. [ ] Switch between Light, Dark, and High Contrast themes
11. [ ] Open the command palette (Ctrl/Cmd+K)
12. [ ] Run a CLI command: `ufop status`

---

## Support Infrastructure

- [ ] Support email/channel documented
- [ ] Known issues list published
- [ ] FAQ prepared for common questions
- [ ] Support team trained on new features
- [ ] Escalation path defined for P0 issues

---

## Launch Gate

| Gate | Status | Notes |
|------|--------|-------|
| No P0 bugs | | |
| No data-loss bugs | | |
| No destructive-sync bugs | | |
| Compatibility corpus passes | | |
| Credential storage approved | | |
| Installers validated (all 3 platforms) | | |
| Documentation complete | | |
| Rollback plan exists | | |
| Crash reporting functional | | |
| Auto-updater verified | | |

**Release Validation: READY FOR GA** when all gates are met.
