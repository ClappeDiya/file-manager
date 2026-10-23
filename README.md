# FileManager — by Clappe

**The cross-platform file manager for power users. One app for local + remote browsing, multi-protocol transfers, real-time sync, and team-grade governance — with your files staying on your machine.**

[Download](https://filemanager.clappe.com) · [Docs](https://filemanager.clappe.com/docs) · [Pricing](https://filemanager.clappe.com/pricing) · [Changelog](https://filemanager.clappe.com/changelog)

---

## What it is

A unified replacement for ForkLift, Transmit, Cyberduck, Rclone-GUI, and the file manager built into your OS — one app that does:

- **Dual-pane browsing** — local + remote side-by-side
- **17 protocols** — SFTP, S3, Google Drive, Dropbox, OneDrive, WebDAV, SMB/CIFS, NFS, FTP/FTPS, Backblaze B2, Azure Blob, Azure Data Lake Gen2, Azure Files, OpenStack Swift, mDNS peer-to-peer, server-to-server relay, and more
- **Crash-safe transfers** — resume from anywhere, integrity-verified with xxHash3 + SHA-256, atomic rename, journal-backed
- **Real-time bidirectional sync** — conflict resolution, redaction rules, scheduled or continuous
- **Local AI assistant** — runs against your own Ollama; no API keys, no cloud calls, no data leaving your box
- **Governance built in** — audit log, version history, encryption-at-rest, master-password vault, lineage tracking
- **CLI** — automate the same operations from your terminal

## Install

```bash
# macOS
brew install --cask ClappeDiya/tap/unified-file-ops

# Windows
winget install UFOP.UnifiedFileOps

# Linux (.deb, .rpm, .AppImage)
# Download from https://filemanager.clappe.com/download
```

All installers are code-signed and auto-update.

Command-line tools (`ufop`, `ufop-server`) with shell completions and man pages: `ufop-<version>-<target>.tar.gz`/`.zip` or `ufop_<version>_amd64.deb` on the [releases page](https://github.com/ClappeDiya/file-manager/releases); `ufop completions bash|zsh|fish|powershell|elvish` prints a completion script for manual installs.

### Verify a download

Every release has `SHA256SUMS`, signed with the same minisign key the app uses for updates (`SHA256SUMS.minisig`) and optionally GPG (`SHA256SUMS.asc`). Put them next to your downloads and run:

```bash
./scripts/verify-release.sh --dir ~/Downloads        # minisign, the ufop CLI or OpenSSL 3; then SHA-256 of each file
ufop verify-release ~/Downloads/SHA256SUMS            # the same check with the CLI
minisign -Vm SHA256SUMS -P RWQOqigyl5kY9Vj4LeRcZ2DMlAEY85jWjDqOZ94gBgYhSd1TOsJ0nxzZ && sha256sum -c --ignore-missing SHA256SUMS
```

## Pricing

| Plan | For | Price |
|---|---|---|
| **Free** | Individuals | Forever free — all 17 connectors, all file ops, all sync, local AI |
| **Business** | Teams | $9.99 / user / month — shared workspaces, policy engine, approval workflows, audit log, SAML SSO, admin console |
| **Enterprise** | Large orgs | Custom — SCIM, SIEM integration, unlimited audit retention, dedicated support |

The desktop app is and always will be free for individuals. We charge for *team coordination*, not for what the file manager does on your laptop.

## Privacy stance

- **No telemetry by default.** Opt-in only.
- **No file contents ever leave your machine** unless *you* configure a transfer to a destination *you* control.
- **Credentials live in your OS keychain.** We never see them.
- **AI runs locally** via Ollama. We don't proxy AI through any cloud service.

## License

Source-available under [PolyForm Shield 1.0.0](LICENSE). Free for any use except building a competing product. Commercial use, modifications, and redistribution are all permitted.

## Contributing & Development

See [CONTRIBUTING.md](CONTRIBUTING.md) for build instructions, architecture overview, and the AI-agent governance setup used by this repo.

To report a security vulnerability privately, see [SECURITY.md](SECURITY.md).
