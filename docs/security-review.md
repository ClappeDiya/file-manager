# UFOP Security Review

Version 0.1.0

---

## Table of Contents

1. [Architecture Overview](#architecture-overview)
2. [Credential Storage](#credential-storage)
3. [Encryption](#encryption)
4. [Transport Security](#transport-security)
5. [Authentication and Authorization](#authentication-and-authorization)
6. [Data at Rest](#data-at-rest)
7. [AI Safety Controls](#ai-safety-controls)
8. [Audit and Compliance](#audit-and-compliance)
9. [Supply Chain Security](#supply-chain-security)
10. [Security Recommendations](#security-recommendations)

---

## Architecture Overview

UFOP is a Tauri-based desktop application with a Rust backend and React/TypeScript frontend:

- **Backend (Rust):** All file operations, network connections, encryption, and credential management run in the Rust process, isolated from the WebView.
- **Frontend (React):** UI rendering only. Communicates with the backend exclusively through Tauri's IPC invoke mechanism.
- **Admin Console (Next.js):** Separate web application for organizational administration.

### Security Model

- The Tauri CSP restricts the WebView to `default-src 'self'` with specific exceptions for asset loading and update checking.
- File system access is mediated through Tauri plugins (`tauri-plugin-fs`, `tauri-plugin-dialog`).
- Network requests are made from the Rust backend, not from the WebView.

---

## Credential Storage

### OS Keychain Integration

UFOP uses the `keyring` crate (v3) with platform-native backends:

| Platform | Backend |
|----------|---------|
| macOS | Apple Keychain (native) |
| Windows | Windows Credential Manager (native) |
| Linux | Secret Service API (GNOME Keyring / KWallet) |

### What Is Stored in the Keychain

- Connection passwords (SFTP, FTP, WebDAV, SMB)
- OAuth tokens (Google Drive, OneDrive, Dropbox)
- API keys for S3, B2, and other cloud services
- Vault passwords (if user opts to save)

### What Is NOT Stored in the Keychain

- Connection profiles (stored in SQLite -- host, port, username, but not passwords)
- Workspace state and preferences
- Transfer history

### Connection Export Safety

When exporting connections to JSON, credentials are **excluded**. Only connection metadata (name, host, port, protocol, username) is exported.

---

## Encryption

### At-Rest Encryption (Vaults)

UFOP provides encrypted vaults for sensitive files:

| Component | Implementation |
|-----------|----------------|
| **Algorithms** | AES-256-GCM and ChaCha20-Poly1305 |
| **Key Derivation** | Argon2id (memory-hard KDF) |
| **Memory Safety** | `zeroize` crate for zeroing sensitive memory after use |

### Encrypt-for-Upload

Files can be encrypted before uploading to cloud storage, ensuring the cloud provider never has access to plaintext data.

### Encryption Policies

Administrators can define policies requiring encryption:
- For specific file types (e.g., `.docx`, `.xlsx`)
- For transfers to specific destinations
- For all external transfers

---

## Transport Security

### TLS Enforcement

UFOP enforces TLS for all supported protocols:

| Protocol | Security |
|----------|----------|
| SFTP | SSH-encrypted (always encrypted) |
| FTPS | TLS 1.2+ required |
| WebDAV | HTTPS enforced (with policy) |
| S3 | HTTPS by default |
| Google Drive | OAuth 2.0 over HTTPS |
| OneDrive | OAuth 2.0 over HTTPS |
| Dropbox | OAuth 2.0 over HTTPS |
| Peer-to-Peer | TLS-encrypted channels |
| Server-to-Server | TLS where supported |

### Transport Security Checking

The `check_transport_security` command evaluates connection configurations and warns about insecure setups. Custom rules can be defined with `check_transport_security_custom`.

### URL Enforcement

The `enforce_https_url` function rewrites HTTP URLs to HTTPS where the server supports it.

---

## Authentication and Authorization

### Desktop Application

- No authentication required for local file operations.
- Connection credentials are retrieved from the OS keychain on demand.
- OAuth 2.0 flows for cloud services use the authorization code grant with PKCE.

### Admin Console

- JWT-based authentication (OAuth 2.0 client credentials for Enterprise tier).
- API key authentication via `X-API-Key` header (Business tier).
- Role-based access control with 5 levels: Super Admin, Org Admin, Manager, User, Viewer.

### Peer Authentication

- Peer discovery uses mDNS/Bonjour on the local network.
- Trust levels: Untrusted (requires approval), Trusted (auto-accept), Blocked (reject all).
- Transfers between peers use TLS-encrypted channels.

---

## Data at Rest

### SQLite Database

The local SQLite database contains:
- Workspace state (pane layout, tabs, view settings)
- Connection profiles (without passwords)
- Transfer history
- Sync pair configurations
- AI chat history and audit log

The database is stored in the OS-specific application data directory and is protected by file system permissions.

### Sensitive Data in Memory

- The `zeroize` crate ensures sensitive data (passwords, encryption keys) is zeroed when dropped.
- Argon2 parameters are configured for security: the derived key is never stored in plaintext.

---

## AI Safety Controls

### Confirmation Gate

All destructive AI-initiated actions require explicit user confirmation before execution. The system checks `ai_check_confirmation_needed` before proceeding with any operation that could modify or delete files.

### Audit Logging

Every AI interaction is logged with:
- Input summary (what the user asked)
- Output summary (what the AI did)
- Whether the user confirmed
- Whether the action was destructive
- Timestamp and metadata

### Feature Toggles

Individual AI features can be disabled:
- Master AI switch
- Error explanations
- Proactive suggestions
- Natural language job creation
- Content analysis (opt-in only)

### Model Routing

AI processing can be configured to use:
- **Local** models only (no data leaves the device)
- **Cloud** models (with user consent and data minimization)

---

## Audit and Compliance

### Desktop Audit Trail

All file operations are recorded in the activity feed with:
- Operation type
- Paths involved
- Timestamp
- Undo capability flag

### Admin Audit Log

The admin console provides a comprehensive audit log with:
- Cryptographic chaining for tamper detection
- Severity levels (info, warning, error, critical)
- User identification and IP tracking
- Date-range querying and export

### Compliance Features

- **Approval Workflows** - Require admin approval for sensitive operations
- **Encryption Policies** - Enforce encryption for specific file types or destinations
- **Transfer Policies** - Restrict transfer destinations and sizes
- **AI Governance** - Control AI feature access and audit all AI actions

---

## Supply Chain Security

### Dependencies

- **Rust backend:** Dependencies are pinned via `Cargo.lock`. Key cryptographic crates (`ring`, `aes-gcm`, `chacha20poly1305`, `argon2`) are well-audited.
- **Frontend:** Dependencies are pinned via `pnpm-lock.yaml`. The `.pnpmfile.cjs` ensures consistent resolution.
- **CI/CD:** GitHub Actions CI runs lint, type checking, tests, and format verification on every push and PR.

### Build Verification

- Frontend type checking: `tsc --noEmit`
- Rust format check: `cargo fmt --all -- --check`
- Rust lint: `cargo clippy`
- Test suites for both frontend and backend

---

## Security Recommendations

### For Users

1. **Keep UFOP updated** - The auto-updater checks for security patches.
2. **Use OS keychain** - Always save passwords in the keychain rather than remembering them manually.
3. **Enable transfer verification** - Use checksum verification for important transfers.
4. **Use encrypted vaults** for sensitive files before uploading to cloud storage.
5. **Review AI suggestions** before accepting them, especially for destructive actions.

### For Administrators

1. **Enable encryption policies** for external transfers.
2. **Set up approval workflows** for large transfers or transfers to external destinations.
3. **Monitor the audit log** regularly for unusual activity.
4. **Configure AI governance** to restrict AI capabilities as appropriate.
5. **Enforce transport security** by setting TLS policies.
6. **Review device compliance** regularly for non-compliant clients.

### For Developers

1. **Never store secrets in code** - Use the keychain integration.
2. **Run security linting** - `cargo clippy` and `eslint` catch common issues.
3. **Update dependencies regularly** - Monitor for CVEs in Cargo and npm dependencies.
4. **Test with `tempfile`** - Use temporary directories for test data, never production paths.
