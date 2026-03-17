# UFOP Troubleshooting Guide

Version 0.1.0

---

## Table of Contents

1. [Installation Issues](#installation-issues)
2. [Startup Problems](#startup-problems)
3. [Connection Issues](#connection-issues)
4. [Transfer Problems](#transfer-problems)
5. [Sync Issues](#sync-issues)
6. [Performance](#performance)
7. [Cross-Platform Compatibility](#cross-platform-compatibility)
8. [Security and Encryption](#security-and-encryption)
9. [AI Assistant](#ai-assistant)
10. [Admin Console](#admin-console)
11. [CLI Issues](#cli-issues)
12. [Logs and Diagnostics](#logs-and-diagnostics)

---

## Installation Issues

### macOS: "App is damaged and can't be opened"

This occurs when the app is downloaded from the internet and macOS Gatekeeper blocks it.

**Fix:**
1. Open System Settings > Privacy & Security
2. Scroll to the "Security" section
3. Click "Open Anyway" next to the UFOP message

### Windows: SmartScreen Warning

Windows Defender SmartScreen may block the installer if the code signing certificate is not yet established.

**Fix:**
1. Click "More info" on the SmartScreen dialog
2. Click "Run anyway"
3. For enterprise deployment, add the UFOP installer hash to your allow list

### Linux: Missing Dependencies

On Debian/Ubuntu systems, the following packages are required:

```bash
sudo apt-get install libwebkit2gtk-4.1-0 libgtk-3-0 libayatana-appindicator3-1
```

For building from source, additional dependencies are needed:

```bash
sudo apt-get install libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libssl-dev
```

---

## Startup Problems

### Application fails to start with database error

**Symptom:** Error message about database initialization failure.

**Cause:** The SQLite database file may be corrupted or the data directory is not writable.

**Fix:**
1. UFOP will automatically fall back to an in-memory database. Data will not persist across restarts in this mode.
2. Check the log output for the specific error.
3. The database is stored in the OS-specific data directory:
   - macOS: `~/Library/Application Support/com.ufop.app/`
   - Windows: `%APPDATA%\com.ufop.app\`
   - Linux: `~/.local/share/com.ufop.app/`
4. If the database is corrupted, rename or delete the `ufop.db` file. A new one will be created on next launch.

### Blank white screen on startup

**Symptom:** The window opens but shows a blank white area.

**Fix:**
1. Wait 5-10 seconds -- the frontend may still be loading.
2. Try resizing the window.
3. Check if the dev tools console shows JavaScript errors (View > Developer Tools if available).
4. Delete the WebView cache:
   - macOS: `~/Library/WebKit/com.ufop.app/`
   - Windows: `%LOCALAPPDATA%\com.ufop.app\EBWebView\`

### Transfer queue not restored after crash

**Symptom:** Active transfers are lost after an unexpected shutdown.

**How UFOP handles this:**
- On normal shutdown, the transfer queue is persisted to the database.
- On startup, UFOP recovers queued and active transfers from the database.
- Active transfers are automatically re-queued (reset to "Queued" status).
- The three-layer transfer journal provides crash recovery for in-progress transfers.

If transfers are still missing, check the database for the `transfer_queue` table.

---

## Connection Issues

### SFTP: "Host key verification failed"

**Cause:** The SSH server's host key doesn't match the known key.

**Fix:**
1. If the server was reinstalled or its keys changed, remove the old key from `~/.ssh/known_hosts`.
2. Reconnect and accept the new host key.
3. For jump host configurations, verify both the jump host and target host keys.

### FTP: Connection times out

**Cause:** Firewall blocking FTP data connections.

**Fix:**
1. Try switching between active and passive mode in connection settings.
2. Passive mode is recommended for most NAT/firewall setups.
3. Ensure the FTP data port range is open if using active mode.

### WebDAV: 401 Unauthorized

**Cause:** Credentials are incorrect or the server requires app-specific passwords.

**Fix:**
1. Verify username and password.
2. For Nextcloud/ownCloud, generate an app-specific password in the web UI.
3. Check if the WebDAV URL is correct (e.g., `https://cloud.example.com/remote.php/dav/files/username/`).

### SMB: Share not discovered

**Cause:** SMB discovery relies on network broadcasts which may be blocked.

**Fix:**
1. Try connecting directly using `smb://hostname/sharename`.
2. Verify the computer is on the same subnet.
3. Check that SMB is enabled on the target machine.
4. On macOS, check System Settings > General > Sharing > File Sharing.

### S3: Access Denied

**Cause:** IAM permissions or bucket policy blocking access.

**Fix:**
1. Verify the access key and secret key are correct.
2. Check IAM permissions include `s3:ListBucket`, `s3:GetObject`, `s3:PutObject`.
3. For S3-compatible services (MinIO, Wasabi), verify the endpoint URL.
4. Check the bucket region matches the configured region.

### Cloud OAuth: "Token expired"

**Cause:** OAuth refresh token has expired or been revoked.

**Fix:**
1. Disconnect the cloud connection.
2. Reconnect and re-authenticate through the OAuth flow.
3. For Google Drive, check the app's access in your Google Account settings.

### Keychain Access Denied

**Cause:** The OS keychain denied access to stored credentials.

**Fix:**
- **macOS:** Open Keychain Access, find the UFOP entry, and click "Always Allow."
- **Windows:** Check Windows Credential Manager for UFOP entries.
- **Linux:** Verify the Secret Service (GNOME Keyring or KWallet) is running.

---

## Transfer Problems

### Transfer stuck at 0%

**Possible causes:**
1. Network connectivity issue -- check your connection.
2. Bandwidth throttle set too low -- check Settings > Transfers > Throttle.
3. Destination is read-only or full.
4. Firewall blocking the transfer.

**Fix:** Cancel the transfer, verify the connection with `ufop connection test`, and retry.

### Transfer fails with checksum mismatch

**Symptom:** Transfer completes but verification reports a checksum mismatch.

**Cause:** Data corruption during transfer (network issue, storage error, or encoding problem).

**Fix:**
1. Retry the transfer with `--resume` to re-transfer only the corrupted portions.
2. If the issue persists, try with `--parallel 1` to use a single stream.
3. Check the source file hasn't changed during transfer.

### Large file transfers fail

**Cause:** Some protocols have file size limits or timeout issues with large files.

**Fix:**
1. For S3/B2, UFOP uses multipart uploads automatically. Verify your timeout settings.
2. For SFTP/FTP, increase the connection timeout in settings.
3. Use `--resume` to recover from network interruptions.
4. Consider using `--parallel 8` for faster throughput on large files.

---

## Sync Issues

### Sync reports "conflict" for every file

**Cause:** Clock skew between source and destination, or timezone differences.

**Fix:**
1. Verify both systems have synchronized clocks (NTP).
2. Run a dry run to inspect the specific conflicts.
3. Consider using checksum-based verification instead of timestamp comparison.

### Filesystem watcher not detecting changes

**Cause:** OS limits on filesystem watchers, or the filesystem doesn't support notification.

**Fix:**
1. **macOS:** Uses FSEvents natively. Should work on all HFS+/APFS volumes.
2. **Linux:** Check `inotify` watch limit: `cat /proc/sys/fs/inotify/max_user_watches`. Increase if needed:
   ```bash
   echo 524288 | sudo tee /proc/sys/fs/inotify/max_user_watches
   ```
3. Network filesystems (NFS, SMB) may not support change notifications. Use scheduled sync instead.

### Sync rollback failed

**Cause:** The pre-sync backup was deleted or moved.

**Fix:**
1. Check the quarantine directory for backup copies.
2. Rollback is only available if the backup from the last sync run is intact.
3. Future runs will create new backups.

---

## Performance

### File list loads slowly for directories with many files

**Fix:**
1. UFOP uses virtual scrolling (via `@tanstack/react-virtual`) to handle large directories. Only visible rows are rendered.
2. If still slow, try switching to List view instead of Grid view.
3. Use filter text to narrow down the visible files.

### High memory usage

**Fix:**
1. Close unused tabs.
2. Reduce the number of active transfers.
3. Disable the preview pane for directories with many large files.
4. Clear the activity feed if it has accumulated many entries.

### Workspace restore takes too long

UFOP is designed to restore workspace state in under 500ms. If it takes longer:

1. Check the database file size. If very large, run `reset_database` to start fresh.
2. Reduce the number of saved searches, favorites, and recent locations.

---

## Cross-Platform Compatibility

### Files renamed unexpectedly during transfer

**What happened:** UFOP detected names incompatible with the destination platform and renamed them.

**What you can do:**
1. Look for a compatibility notification in the Activity feed.
2. Use the "Restore Original Names" guided wizard to undo the renames.
3. Adjust compatibility settings if you only transfer between specific platforms.

### Unicode normalization warnings

**Cause:** macOS uses NFD (decomposed) Unicode while Windows/Linux use NFC (composed). This means the same filename can appear different on different systems.

**Fix:** UFOP normalizes filenames during transfer. The original names are preserved and can be restored.

---

## Security and Encryption

### Cannot unlock vault

**Cause:** Incorrect password or vault file corruption.

**Fix:**
1. Verify the password is correct (no extra spaces).
2. Check that the vault file hasn't been moved or renamed.
3. Vault encryption uses Argon2 for key derivation, which is intentionally slow to prevent brute force.

### Transport security warning

**Symptom:** UFOP warns about insecure transport.

**Cause:** A connection is using unencrypted FTP or HTTP instead of FTPS/SFTP or HTTPS.

**Fix:**
1. Switch to SFTP instead of FTP, or enable TLS for the FTP connection.
2. Use HTTPS URLs for WebDAV connections.
3. In enterprise environments, use encryption policies to enforce transport security.

---

## AI Assistant

### AI responses are slow or unavailable

**Cause:** The AI model may be loading or the local model is resource-constrained.

**Fix:**
1. Check model routing in AI settings (Settings > AI > Model Routing).
2. Local models require sufficient RAM and CPU.
3. The AI gracefully falls back to structured error messages when unavailable.

### AI suggests incorrect actions

**Fix:**
1. Dismiss the suggestion.
2. Destructive actions always require confirmation before execution.
3. Review the AI audit log for previous interactions.
4. Disable specific AI features in Settings > AI > Feature Toggles if needed.

---

## Admin Console

### Admin console shows blank page

**Fix:**
1. Verify the admin server is running: `pnpm admin:dev` or check production deployment.
2. Clear browser cache and cookies.
3. Check the browser console for JavaScript errors.

### API returns 401 for admin endpoints

**Fix:**
1. Re-authenticate via the login page.
2. Check that your API key or JWT token hasn't expired.
3. Verify the admin role permissions include the requested resource.

---

## CLI Issues

### CLI cannot connect to server

**Fix:**
1. Check `UFOP_API_URL` environment variable or `--api-url` flag.
2. Test connectivity: `curl <API_URL>/api/health`
3. Verify the server is running and accessible from your network.
4. Check for proxy settings that may interfere.

### CLI output is garbled

**Cause:** Terminal doesn't support ANSI colors or Unicode.

**Fix:**
1. Use `--format json` or `--format yaml` for machine-readable output.
2. Pipe output through a pager: `ufop status | less -R`
3. Set `NO_COLOR=1` to disable colors.

---

## Logs and Diagnostics

### Enabling Verbose Logging

**Desktop app:** Set the `RUST_LOG` environment variable before launching:

```bash
RUST_LOG=debug /path/to/ufop
RUST_LOG=trace /path/to/ufop  # Maximum verbosity
```

**CLI:** Use the `-v` flags:

```bash
ufop status -v      # Info level
ufop status -vv     # Debug level
ufop status -vvv    # Trace level
```

### Log Locations

- **Desktop:** Logs are written to stderr. On macOS, check Console.app. On Linux, check `journalctl`.
- **CLI:** Logs are written to stderr by default.
- **Admin Console:** Check the Next.js server logs.

### Reporting Issues

When reporting a bug, include:

1. UFOP version (`ufop status --json`)
2. Operating system and version
3. Steps to reproduce
4. Relevant log output (with sensitive data redacted)
5. Error messages (exact text)
