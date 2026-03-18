# UFOP Migration Guide

Version 0.1.0

---

## Table of Contents

1. [Migrating from Other File Managers](#migrating-from-other-file-managers)
2. [Importing Connections](#importing-connections)
3. [Data Migration Wizards](#data-migration-wizards)
4. [Upgrading UFOP](#upgrading-ufop)
5. [Database Migration](#database-migration)
6. [Platform-Specific Notes](#platform-specific-notes)

---

## Migrating from Other File Managers

### From FileZilla

UFOP supports SFTP, FTP, and FTPS connections. To migrate your FileZilla sites:

1. In FileZilla, export your Site Manager entries (File > Export > Export Site Manager entries)
2. In UFOP, use the connection import feature or manually recreate connections
3. UFOP stores credentials in the OS keychain rather than in XML files, providing better security

### From Cyberduck

1. Cyberduck bookmarks can be exported as `.duck` files
2. Manually recreate connections in UFOP using the same server details
3. For S3 connections, use the same endpoint URL, access key, and secret key

### From WinSCP

1. Export WinSCP sessions from the Login dialog
2. Recreate the SFTP/FTP/SCP connections in UFOP
3. Note: UFOP supports SFTP and FTP but does not currently support the SCP-only protocol mode

### From Total Commander / Double Commander

1. UFOP provides a similar dual-pane layout in Advanced mode
2. Keyboard shortcuts differ; refer to the [User Guide](./user-guide.md) for the UFOP shortcut map
3. Plugin-based protocols need to be recreated using UFOP's built-in connectors

---

## Importing Connections

### Batch Import

UFOP supports importing connection profiles from JSON:

```json
[
  {
    "name": "Production SFTP",
    "protocol": "sftp",
    "host": "sftp.example.com",
    "port": 22,
    "username": "deploy",
    "auth_type": "key"
  },
  {
    "name": "Backup S3",
    "protocol": "s3",
    "host": "s3.amazonaws.com",
    "settings": {
      "bucket": "my-backup",
      "region": "us-east-1"
    }
  }
]
```

Import via the desktop app:
1. Open Cloud & Servers section
2. Click the import button
3. Select your JSON file

Import via the CLI:
```bash
ufop connection add prod-sftp sftp://deploy@sftp.example.com
```

### Exporting Connections

Export all connections for backup or transfer to another machine:

1. Open Cloud & Servers > Export Connections
2. Choose format (JSON)
3. Credentials are **not** included in exports for security

---

## Data Migration Wizards

UFOP includes guided wizards for common migration scenarios:

### Old Drive to New Drive

Transfer all data from an old drive to a new one:

1. Launch the "Migrate Drive" wizard from Simple mode sidebar or command palette
2. Select the source drive (old)
3. Select the destination drive (new)
4. UFOP runs a preflight check (space, compatibility, permissions)
5. Review the transfer plan
6. Execute with verification

### Computer to Computer

Transfer files and settings from an old computer:

1. Install UFOP on both computers
2. Ensure both are on the same network
3. Launch "Migrate from Computer" wizard on the new computer
4. UFOP discovers the old computer via mDNS
5. Select which files/folders to transfer
6. Transfer proceeds over TLS-encrypted peer connection

### Copy to External Drive

Copy important files to a USB drive, SD card, or external disk:

1. Launch "Copy to External Drive" wizard
2. Select files or folders
3. Select the external drive (auto-detected)
4. UFOP runs a preflight check
5. Execute with optional verification

---

## Upgrading UFOP

### Auto-Update

UFOP includes built-in update functionality:

- The app checks for updates from `https://releases.ufop.app`
- A dialog prompts when an update is available
- Updates are verified against a public key before installation
- On Windows, updates can be installed in passive mode (no user interaction required)

### Manual Update

1. Download the new version from the releases page
2. Install over the existing version
3. Your settings, connections, and data are preserved (stored in the OS data directory)

### Update Channels

- **Stable** - Production-ready releases
- **GitHub Releases** - Alternative update source at `https://github.com/ufop/unified-file-ops/releases`

---

## Database Migration

UFOP uses SQLite for local data storage. The database is automatically migrated when upgrading to a new version.

### What Gets Migrated

- Workspace state (pane layout, tabs, view settings)
- Connection profiles (credentials remain in the OS keychain)
- Transfer history
- Sync pair configurations
- User preferences

### Manual Database Operations

The database is located at:

- **macOS:** `~/Library/Application Support/com.ufop.app/ufop.db`
- **Windows:** `%APPDATA%\com.ufop.app\ufop.db`
- **Linux:** `~/.local/share/com.ufop.app/ufop.db`

To reset the database:
1. Use the `reset_database` command from the app
2. Or delete the `ufop.db` file and restart the application

### Backup Before Upgrade

Before major version upgrades, back up:
1. The database file
2. Your exported connections JSON
3. Your sync pair configurations (exportable via CLI)

---

## Platform-Specific Notes

### macOS

- Minimum supported version: macOS 10.15 (Catalina)
- App is bundled as a `.dmg` with `.app` inside
- Uses native Keychain for credential storage
- FSEvents for filesystem watching (sync)
- Notarization may be required for distribution outside the App Store

### Windows

- Installers available in WiX (`.msi`) and NSIS (`.exe`) formats
- Supports 8 languages in the installer UI
- Uses Windows Credential Manager for credential storage
- WebView2 is required (automatically installed if missing)
- Digest algorithm: SHA-256 for installer verification

### Linux

- Available as `.deb`, `.rpm`, and `.AppImage`
- Requires `libwebkit2gtk-4.1-0`, `libgtk-3-0`, and `libayatana-appindicator3-1`
- Uses `secret-service` (GNOME Keyring or KWallet) for credential storage
- `inotify` for filesystem watching; check watch limits for large directory trees
