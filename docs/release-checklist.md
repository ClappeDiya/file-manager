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

- [ ] Installers are Authenticode-signed (see "Windows signing and winget")
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

## Building and Publishing (local, no GitHub Actions)

Releases are built and published from the release machine(s) with `scripts/release-local.sh`; there is no CI service. Every packaging step is a local script with a dry-run mode, and `scripts/test-release-scripts.sh` tests them offline (plans on simulated macOS/Windows/Linux hosts, CLI packages, winget and Homebrew generation, signing and verification with throwaway keys).

### Target matrix

`release-local.sh` builds **every target the host can build** and reports what it skipped and why. See it first:

```bash
./scripts/release-local.sh --print-plan                 # matrix, artifacts, signing, upload set
./scripts/release-local.sh --version 1.4.0 --dry-run    # every command, nothing run
./scripts/release-local.sh --version 1.4.0 --no-publish # build + stage + sign, no tag / release
./scripts/release-local.sh --version 1.4.0              # build, sign, verify, tag, gh release create
```

| Target | Built on | Bundles |
|---|---|---|
| `universal-apple-darwin` | a Mac | `.dmg`, `.app.tar.gz` (+ updater `.sig`) |
| `x86_64-unknown-linux-gnu` / `aarch64-unknown-linux-gnu` | Linux of the same architecture (WebKitGTK is not cross-linked) | `.deb`, `.rpm`, `.AppImage` |
| `x86_64-pc-windows-msvc` | Windows (Git Bash): MSI + NSIS; or Linux/macOS with `--windows-cross xwin` (`cargo-xwin`, `makensis`, `clang-cl`, `lld-link`): NSIS only | `.msi`, `-setup.exe` (+ updater `.sig`) |

`--target T` (repeatable) builds only the named targets and fails early, with the reason, for one this host cannot build. `--windows` requires the Windows target, `--no-windows` leaves it out. A full release therefore runs once per machine (Mac, Linux, Windows): run `--no-publish` on each, copy every machine's `release-artifacts/vX.Y.Z/` into one folder, run `scripts/release-checksums.sh --dir <folder>` there (one `SHA256SUMS` for the whole release), then `gh release create vX.Y.Z <folder>/*`. A single-machine release just runs the script without `--no-publish`.

### Windows signing and winget

- Signing is optional and set by the environment: `WINDOWS_SIGN_CERT_THUMBPRINT` (a certificate in the Windows store, e.g. on a token) or `WINDOWS_SIGN_CERT_FILE` + `WINDOWS_SIGN_CERT_PASSWORD` (a `.pfx`); `WINDOWS_SIGN_TIMESTAMP_URL` (default DigiCert). It is passed to Tauri as `bundle.windows.signCommand`, so the app, the MSI and the NSIS installer are signed during bundling, before the updater signatures are made. On Windows the command is `packaging/windows/sign.ps1` (signtool, then `signtool verify /pa`); cross builds use `packaging/windows/sign.sh` (osslsigncode). Secrets stay in the environment, never in the Tauri config. `ufop.exe` and `ufop-server.exe` are signed the same way.
- [ ] `--print-plan` says "signed with …" for the Windows target
- [ ] Right-click → Properties → Digital Signatures on the installer shows the certificate and a timestamp
- winget: when `FileManager-<version>-x64-setup.exe` was built, `packaging/windows/winget/generate-manifest.sh` writes `UFOP.UnifiedFileOps.yaml`, `.installer.yaml` (NSIS, plus the MSI when present, with their SHA-256) and `.locale.en-US.yaml` (manifest schema 1.6) into the release. Submit them with `wingetcreate submit <folder>` (or a pull request to microsoft/winget-pkgs under `manifests/u/UFOP/UnifiedFileOps/<version>/`).
- [ ] `winget validate <folder>` passes on a Windows machine

### CLI packages, completions and man pages

For each built target, `packaging/linux/package-cli.sh` packages `ufop` and `ufop-server` with shell completions (bash, zsh, fish, PowerShell, Elvish) and man pages (`ufop*.1`, and `ufop-server*.8` in section 8 so they never collide with `ufop server …` pages): `ufop-<version>-<target>.tar.gz` (`.zip` for Windows) with an `install.sh` (`PREFIX=/usr/local`), and on Linux `ufop_<version>_<arch>.deb` installing `/usr/bin`, `/usr/share/bash-completion/completions/`, `/usr/share/zsh/vendor-completions/`, `/usr/share/fish/vendor_completions.d/` and `/usr/share/man/`. Completions and man pages are generated by a `ufop` built for the release machine, so cross-built packages have them too. On macOS the universal tarball also yields `ufop.rb` (`packaging/macos/generate-formula.sh`), a Homebrew formula that installs the binaries, completions and man pages; commit it to the tap as `Formula/ufop.rb`.

- [ ] `dpkg -i ufop_<version>_amd64.deb`, then `ufop backup <TAB>` completes in a new bash
- [ ] `man ufop-backup-run` and `man 8 ufop-server` open

### Signed checksums

`scripts/release-checksums.sh` (run by `release-local.sh` after staging) writes `SHA256SUMS` over every file of the release and signs it:

- `SHA256SUMS.minisig`: minisign format, with the **updater key** (`TAURI_SIGNING_PRIVATE_KEY` / `_PASSWORD`, through `tauri signer sign`), or `--minisign-key FILE` with the minisign tool. Users verify with the same public key the app uses for updates (`src-tauri/tauri.conf.json` → `plugins.updater.pubkey`, key id `F5189997328AA80E`).
- `SHA256SUMS.asc`: with `--gpg-key KEYID` (or `UFOP_RELEASE_GPG_KEY`), a detached ASCII-armoured GPG signature. Publish the public key alongside the release notes.

Before anything is uploaded the script runs `scripts/verify-release.sh --require-all` on the staged folder, exactly as a user would. Both files are part of the upload set.

- [ ] `SHA256SUMS`, `SHA256SUMS.minisig` (and `.asc`) are attached to the GitHub release
- [ ] On a clean machine: download two assets plus the three files, run `verify-release.sh` (or `ufop verify-release SHA256SUMS`)

### Docker images

`scripts/docker-build.sh` builds `docker/ufop-server.Dockerfile` for `linux/amd64,linux/arm64` with a buildx builder (docker-container driver) and publishes **one manifest list** to GHCR (`ghcr.io/clappediya/ufop-server`, login with `gh auth token`, like `build-push-marketing.sh`). Tags: `sha-<commit>`, `v<version>` when HEAD is tagged, plus `--tag …`.

```bash
./scripts/docker-build.sh --dry-run --push                  # plan and commands
./scripts/docker-build.sh --push --tag latest                # both platforms, one manifest list
./scripts/docker-build.sh --platforms linux/amd64            # one platform, loaded into the local daemon
./scripts/docker-build.sh --push --append-node ssh://arm64-host   # arm64 built natively on another machine
```

The web UI stage runs once on the build machine's platform (its output is the same for every architecture); the server is compiled natively for each platform. A foreign platform runs under QEMU (`tonistiigi/binfmt` is installed when the builder lacks it). That is the robust choice because `ufop-server` links GTK/WebKitGTK through the Tauri library, which a cross-compiler would need a complete arm64 sysroot for, but emulation is slow: expect an emulated arm64 release build to take several times as long as a native one (roughly 4–8× on typical hardware; an hour or more from a cold cache). Cargo registry and target caches are kept per architecture (`--mount=type=cache`), so later builds are much faster; `--append-node` with a native arm64 machine avoids emulation entirely.

- [ ] `docker buildx imagetools inspect ghcr.io/clappediya/ufop-server:v<version>` lists `linux/amd64` and `linux/arm64`
- [ ] `docker run --rm --platform linux/arm64 ghcr.io/clappediya/ufop-server:v<version> --version` prints the version

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
