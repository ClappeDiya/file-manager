# UFOP API Reference

Version 0.1.0 | OpenAPI 3.0

---

## Overview

The UFOP Admin API is a REST API for administration, transfers, sync, and governance. The live OpenAPI 3.0 specification is available at `GET /api/openapi`.

### Base URL

```
/api
```

### Authentication

All endpoints (except `/health` and `/openapi`) require authentication via one of:

- **Bearer Token** (JWT) - OAuth 2.0 client credentials. Pass as `Authorization: Bearer <token>`.
- **API Key** - Pass as `X-API-Key: <key>` header.

### Common Response Codes

| Code | Description |
|------|-------------|
| 200 | Success |
| 201 | Created |
| 400 | Bad Request - Invalid parameters |
| 401 | Unauthorized - Missing or invalid credentials |
| 403 | Forbidden - Insufficient permissions |
| 404 | Not Found |
| 409 | Conflict - Resource already exists |
| 429 | Too Many Requests - Rate limited |
| 500 | Internal Server Error |

### Error Format

```json
{
  "error": "Human-readable error message",
  "code": "ERROR_CODE"
}
```

---

## System

### Health Check

```
GET /api/health
```

No authentication required.

**Response 200:**

```json
{
  "status": "healthy",
  "version": "0.1.0",
  "uptime_seconds": 86400
}
```

### OpenAPI Spec

```
GET /api/openapi
```

No authentication required. Returns the full OpenAPI 3.0 specification as JSON.

---

## Users

### List Users

```
GET /api/users
```

**Query Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `role` | string | Filter by role: `super_admin`, `org_admin`, `manager`, `user`, `viewer` |
| `search` | string | Search by name or email |
| `active` | boolean | Filter by active status |

**Response 200:**

```json
[
  {
    "id": "usr_abc123",
    "email": "user@example.com",
    "name": "John Doe",
    "role": "user",
    "isActive": true,
    "createdAt": "2024-01-15T09:30:00Z"
  }
]
```

### Create User

```
POST /api/users
```

**Request Body:**

```json
{
  "email": "new@example.com",
  "name": "Jane Doe",
  "role": "user"
}
```

**Response 201:** User object.

**Response 409:** User with that email already exists.

---

## Roles

### List Roles

```
GET /api/roles
```

**Response 200:** Array of role objects with name, description, and permissions.

---

## Devices

### List Devices

```
GET /api/devices
```

**Response 200:** Array of device objects with:

```json
{
  "id": "dev_xyz",
  "deviceName": "MacBook Pro",
  "userName": "John Doe",
  "os": "macOS 14.2",
  "clientVersion": "0.1.0",
  "status": "online",
  "lastSeenAt": "2024-01-20T14:30:00Z",
  "policyCompliance": {
    "isCompliant": true,
    "violations": []
  }
}
```

---

## Policies

### List Policies

```
GET /api/policies
```

**Response 200:** Array of policy objects.

### Create Policy

```
POST /api/policies
```

**Request Body:**

```json
{
  "name": "Require Encryption for External Transfers",
  "type": "transfer",
  "description": "All transfers to external destinations must use encryption",
  "rules": [
    {
      "condition": "destination.external == true",
      "action": "require_encryption"
    }
  ],
  "enforcementMode": "enforce",
  "isActive": true
}
```

**Response 201:** Created policy object.

---

## Approvals

### List Approvals

```
GET /api/approvals
```

**Response 200:** Array of approval requests.

```json
{
  "id": "appr_123",
  "operation": "Transfer to external S3 bucket",
  "requestedByName": "John Doe",
  "requestedAt": "2024-01-20T10:00:00Z",
  "state": "pending",
  "fileCount": 45,
  "totalSize": 1073741824,
  "sourcePath": "/shared/reports",
  "destPath": "s3://external-bucket/reports"
}
```

### Create Approval

```
POST /api/approvals
```

**Request Body:**

```json
{
  "operation": "Transfer description",
  "fileCount": 10,
  "totalSize": 5242880,
  "sourcePath": "/source",
  "destPath": "/dest"
}
```

---

## Audit

### Query Audit Log

```
GET /api/audit
```

**Query Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `action` | string | Filter by event type |
| `user` | string | Filter by user ID or name |
| `from` | datetime | Start date (ISO 8601) |
| `to` | datetime | End date (ISO 8601) |
| `limit` | integer | Results per page (default: 50) |
| `offset` | integer | Pagination offset (default: 0) |

**Response 200:**

```json
[
  {
    "id": "aud_abc",
    "eventType": "transfer.completed",
    "description": "Transferred 45 files to S3",
    "userName": "John Doe",
    "userId": "usr_abc123",
    "timestamp": "2024-01-20T14:30:00Z",
    "severity": "info",
    "ipAddress": "192.168.1.100",
    "metadata": {}
  }
]
```

---

## Connectors

### List Connectors

```
GET /api/connectors
```

**Response 200:** Array of connector objects.

### Create Connector

```
POST /api/connectors
```

**Request Body:**

```json
{
  "name": "Production SFTP",
  "protocol": "sftp",
  "host": "sftp.example.com",
  "port": 22,
  "username": "deploy",
  "authType": "key"
}
```

### Get Connector

```
GET /api/connectors/{id}
```

### Update Connector

```
PATCH /api/connectors/{id}
```

### Delete Connector

```
DELETE /api/connectors/{id}
```

### Connector Actions

```
POST /api/connectors/{id}
```

**Request Body:**

```json
{
  "action": "test"
}
```

Actions: `test`, `connect`, `disconnect`.

---

## Transfers

### List Transfers

```
GET /api/transfers
```

**Query Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `status` | string | `queued`, `active`, `paused`, `completed`, `failed`, `cancelled` |
| `priority` | string | `high`, `normal`, `low` |
| `limit` | integer | Results per page (default: 50) |
| `offset` | integer | Pagination offset (default: 0) |

**Response 200:**

```json
[
  {
    "id": "xfr_abc",
    "source": "/local/path",
    "destination": "s3://bucket/path",
    "status": "active",
    "progress": 0.65,
    "total_bytes": 1073741824,
    "transferred_bytes": 697932185,
    "speed_bps": 52428800,
    "priority": "normal",
    "created_at": "2024-01-20T10:00:00Z",
    "updated_at": "2024-01-20T10:15:00Z",
    "error_message": null,
    "verify_checksum": true
  }
]
```

### Create Transfer

```
POST /api/transfers
```

**Request Body:**

```json
{
  "source": "/local/file.zip",
  "destination": "s3://bucket/file.zip",
  "priority": "high",
  "verify_checksum": true
}
```

**Response 201:** Transfer object.

### Get Transfer

```
GET /api/transfers/{id}
```

### Update Transfer

```
PATCH /api/transfers/{id}
```

**Request Body:**

```json
{
  "action": "pause"
}
```

Actions: `pause`, `resume`, `cancel`.

### Delete Transfer

```
DELETE /api/transfers/{id}
```

Cancels and removes the transfer.

---

## Sync

### List Sync Pairs

```
GET /api/sync
```

**Response 200:**

```json
[
  {
    "id": "sync_abc",
    "name": "Documents Backup",
    "source": "/home/user/documents",
    "destination": "s3://backup/documents",
    "direction": "source-to-dest",
    "schedule": "0 2 * * *",
    "status": "idle",
    "last_run": "2024-01-20T02:00:00Z",
    "files_synced": 1250,
    "bytes_synced": 5368709120
  }
]
```

### Create Sync Pair

```
POST /api/sync
```

**Request Body:**

```json
{
  "name": "My Sync",
  "source": "/path/a",
  "destination": "/path/b",
  "direction": "bidirectional",
  "schedule": "0 */6 * * *"
}
```

### Get Sync Pair

```
GET /api/sync/{id}
```

### Update Sync Pair

```
PATCH /api/sync/{id}
```

**Request Body for triggering a run:**

```json
{
  "action": "run"
}
```

### Delete Sync Pair

```
DELETE /api/sync/{id}
```

---

## Webhooks

### List Webhooks

```
GET /api/webhooks
```

**Response 200:**

```json
[
  {
    "id": "wh_abc",
    "url": "https://example.com/webhook",
    "events": ["transfer.completed", "sync.failed", "policy.violation"],
    "is_active": true,
    "created_at": "2024-01-15T09:00:00Z",
    "last_triggered": "2024-01-20T14:30:00Z",
    "failure_count": 0
  }
]
```

### Register Webhook

```
POST /api/webhooks
```

**Request Body:**

```json
{
  "url": "https://example.com/webhook",
  "events": ["transfer.completed", "sync.failed"]
}
```

### Available Webhook Events

| Event | Description |
|-------|-------------|
| `transfer.completed` | A transfer finished successfully |
| `transfer.failed` | A transfer failed |
| `sync.completed` | A sync run completed |
| `sync.failed` | A sync run failed |
| `policy.violation` | A policy was violated |
| `approval.requested` | A new approval request was created |
| `approval.resolved` | An approval was approved or denied |
| `user.login` | A user logged in |
| `device.registered` | A new device connected |

### Get Webhook

```
GET /api/webhooks/{id}
```

### Update Webhook

```
PATCH /api/webhooks/{id}
```

### Delete Webhook

```
DELETE /api/webhooks/{id}
```

---

## AI Governance

### Get AI Settings

```
GET /api/ai-governance?view=settings
```

**Response 200:**

```json
{
  "ai_enabled": true,
  "auto_suggestions": true,
  "natural_language_commands": true,
  "require_confirmation_for_destructive": true,
  "max_files_per_action": 100,
  "allowed_actions": ["rename", "copy", "move", "sync"],
  "audit_retention_days": 90
}
```

### Get AI Audit Log

```
GET /api/ai-governance?view=audit
```

### Get AI Feature Toggles

```
GET /api/ai-governance?view=toggles
```

### Update AI Settings

```
POST /api/ai-governance
```

**Request Body:**

```json
{
  "ai_enabled": true,
  "auto_suggestions": false,
  "max_files_per_action": 50
}
```

---

## ufop-server REST API (`/api/v1`)

The headless backup server (`ufop-server`, also `ufop server` in the CLI) has its own versioned API, described by a live **OpenAPI 3.1** document at `GET /api/v1/openapi.json` (public, like `/api/v1/health`). Load it into Swagger UI, Postman or a client generator.

- **Authentication**: `Authorization: Bearer <token>` with an access token from `POST /api/v1/auth/login` (refreshed through the `ufop_refresh` cookie), an automation token, or an API token (`ufop_pat_…`); or reverse-proxy pre-authentication (`X-UFOP-Preauth`).
- **API token scopes**: `read` < `run` < `write` < `admin`. Each operation's `x-required-scope` in the document comes from the same function that enforces it: GETs need `read`; starting backups, operations, cancelling tasks and the scheduler need `run`; other changes need `write`; `/auth/*`, `/rpc/*`, `/oauth/*` and changing settings need `admin`.
- **Errors**: `{error_type, message, advice}`; 401 (not signed in, invalid token or too narrow a scope), 403 (client address outside `--allowed-cidrs`), 421 (host not in `--allowed-hostnames`), 404, 409, 429 (sign-in throttling).
- Areas: `auth` (sign-in, tokens, password), `jobs` (CRUD, run, operations including `upgrade_format`, `reshard` and `format_info`, `PUT /jobs/{id}/passphrase` to change an encrypted job's passphrase, import/export, destination test), `browse` (versions, files, search, compare), `tasks` (tasks, run history, scheduler), `oauth` (cloud authorization broker) and `general` (health, status, modules, settings, filesystem, `rpc/{command}` for any desktop IPC command).

### Share portal

Start the server with `--share-root <folder>` (repeatable, or `UFOP_SERVER_SHARE_ROOTS`) to share files and folders under it by link; without it sharing is off. Links are created in the web UI (*Share links*), with `ufop server shares create <path>` or `POST /api/v1/shares`, and open at `/s/<token>` without an account:

- **download** — a file, or a folder to browse (subfolders included) and download from; **upload** — a drop box: visitors upload into the folder and see nothing in it; **both**.
- Options: password (8+ characters; argon2; 10 wrong tries lock the link for 15 minutes), expiry in days, download limit, largest upload per file (default 1 GB).
- Only a hash of the token is stored, so the link is shown once. Paths are confined to the shared folder after resolving symbolic links, dot-files are never listed or served, downloads are sent as attachments, uploads never overwrite (`name (2).ext`), and pages carry a strict Content-Security-Policy. The server's `--allowed-cidrs` and host checks apply to links too.
- Every view, download, upload, refusal and wrong password is logged per link (`GET /api/v1/shares/{id}/events`, `ufop server shares log <id>`); links can be turned off at any time.

### Folder policies

Folders under `--share-root` can carry a policy (web UI *Share links → Folder policies*, `ufop server folders`, `/api/v1/folder-policies`); subfolders inherit, and for each setting the nearest folder that sets it wins:

- **Lock** — the folder takes no uploads (existing upload links stop accepting files) and no new upload links.
- **Accepted / blocked extensions** and the **largest upload** (the smaller of this and the link's limit applies). Refused files are skipped and reported; the rest of an upload goes through.
- **Link rules** — links to the folder need a password, and/or must expire within N days.
- **Intake order** — uploads keep their names (`name (2).ext` on a clash), get an arrival-time prefix (`2026-10-22T16-30-01.123Z name.pdf`), or a per-folder sequence number (`000042-name.pdf`), so listing by name lists by arrival.
- **Copy** a folder's policy to another folder (`POST /api/v1/folder-policies/copy`); an empty policy removes a folder's own settings.

A test fails the build when a route is added without an OpenAPI entry (or an entry outlives its route), and another checks every documented scope against the running server.

---

### WebDAV (read-only)

Start the server with `--dav-root <folder>` (repeatable, or `UFOP_SERVER_DAV_ROOTS`) to serve folders read-only over WebDAV at `/dav/<folder name>/`; without it WebDAV is off. Any WebDAV client works (Windows *Map network drive*, macOS *Connect to Server*, rclone `webdav`, Cyberduck, `davfs2`):

- **Methods:** `OPTIONS`, `PROPFIND` with `Depth: 0` or `1` (infinity is refused with `propfind-finite-depth`), `GET`/`HEAD` with single byte ranges, `ETag` and `If-None-Match`. `PUT`, `DELETE`, `MKCOL`, `COPY`, `MOVE`, `PROPPATCH`, `LOCK` and every other method get `405 Method Not Allowed`.
- **Sign-in:** HTTP Basic with any user name and the server password or an API token (`ufop_pat_…`, any scope) as the password, or `Authorization: Bearer`. An accepted password is remembered for a minute; changing the password or resetting tokens forgets it, and a revoked API token stops working at once.
- **Network:** `--allowed-cidrs` applies as everywhere; `--dav-allowed-cidrs` (or `UFOP_SERVER_DAV_ALLOWED_CIDRS`) restricts WebDAV further. Basic sends the password with every request, so use `--https` or a TLS reverse proxy when the server is reachable from other machines (the server warns otherwise).
- **Confinement:** paths are resolved after following symlinks and must stay inside the folder; dot-files and links pointing outside are neither listed nor served.

### MCP over HTTP (OAuth)

Start the server with `--mcp-root <folder>` (repeatable, or `UFOP_SERVER_MCP_ROOTS`) to offer the same file tools as `ufop mcp` to remote AI clients at `POST /mcp` (Streamable HTTP; JSON responses; `GET`/`DELETE` → 405). Read-only unless `--mcp-allow-write`; behind a proxy set `--public-url https://files.example.com` so the OAuth metadata names the right origin.

- **Authorization** follows the MCP authorization spec, so clients such as Claude connect by URL alone: an unauthenticated request gets `401` with `WWW-Authenticate: Bearer resource_metadata="…/.well-known/oauth-protected-resource"`; the client reads `/.well-known/oauth-authorization-server`, registers itself at `/oauth2/register` (public clients; redirect URIs must be `https` or `http` on `localhost`/`127.0.0.1`), and sends the user to `/oauth2/authorize`, a consent page where the server password approves it. PKCE (`S256`) is required; scopes are `mcp:read` and `mcp:write` (granted only with `--mcp-allow-write`); the authorization response carries `iss`.
- **Tokens:** `/oauth2/token` exchanges the code (single use, two minutes) for a one-day access token and a 30-day refresh token. Refresh tokens rotate: each use revokes it and the access token it issued, and a second use fails. `/oauth2/revoke` revokes either kind. Access tokens are API tokens named `MCP: <client name>` with the `read` or `write` scope — they appear in the API token list and can be revoked there; API tokens and sessions work on `/mcp` too.
- **Tools:** write tools exist only with `--mcp-allow-write` and a token that may write. Paths are confined to the roots exactly as with `ufop mcp`. Requests whose `Origin` is another site (DNS rebinding) are refused; use `--https` or a TLS proxy when reachable from other machines (the server warns).

### File gateways

On-premises agents (`ufop gateway serve`) serve folders through the server without an inbound port. An agent long-polls `POST /api/v1/gateway/poll` with an API token of the **write** scope, receives jobs (`list`, `download`, `upload`) and streams its answers to `POST /api/v1/gateway/jobs/{id}/reply`. For uploads it reads the client's data from `GET /api/v1/gateway/jobs/{id}/body`. Nothing is stored on the server.

| Client call | Scope | |
|---|---|---|
| `GET /api/v1/gateways` | read | Gateways, nodes (host, address, roots, writable, last seen, jobs) |
| `GET /api/v1/gateways/{name}/files?path=/root/dir` | read | One folder (`/` = the roots) |
| `GET /api/v1/gateways/{name}/download?path=/root/file` | read | The file, streamed |
| `PUT /api/v1/gateways/{name}/upload?path=/root/file` | write | The request body becomes the file (writable gateways only) |

Nodes that share a gateway name form a pool: each job goes to the first node that polls, read-only nodes leave uploads to writable ones, and a stopping agent leaves the pool at once (others drop out after 60 s without polling). With no node online, a request fails with `503`; a job no node answers within 60 s fails with `504`. Agent errors map to `404`, `403` or `400`.

## Tauri IPC Commands

The desktop application uses Tauri IPC for communication between the frontend (React/TypeScript) and backend (Rust). These are not REST endpoints but are documented here for extension developers.

### Categories

| Category | Commands |
|----------|----------|
| **System** | `greet`, `get_app_version`, `get_platform_info` |
| **Filesystem** | `list_directory` |
| **State** | `save_workspace_state`, `load_workspace_state`, `get_config`, `set_config`, `reset_database` |
| **Transfers** | `enqueue_transfer`, `pause_transfer`, `resume_transfer`, `cancel_transfer`, `list_transfers`, `get_transfer`, `set_transfer_priority`, `reorder_transfer`, `set_global_throttle`, `set_connection_throttle`, `get_throttle_config`, `retry_transfer`, `retry_all_failed`, `verify_transfer`, `compute_checksum`, `search_transfer_history`, `export_transfer_history` |
| **Connections** | `save_connection`, `list_connections`, `get_connection`, `delete_connection`, `test_connection`, `search_connections`, `create_connection_group`, `export_connections`, `import_connections` |
| **Connectors** | `connector_connect`, `connector_disconnect`, `connector_list_remote`, `connector_is_connected`, `connector_list_protocols` |
| **Drives** | `smb_discover_shares`, `smb_list_shares`, `nfs_list_exports`, `detect_drives`, `transfer_preflight`, `eject_drive` |
| **Sync** | `create_sync_pair`, `delete_sync_pair`, `list_sync_pairs`, `update_sync_pair`, `run_sync`, `get_sync_health`, `sync_dry_run`, `get_sync_conflicts`, `resolve_sync_conflict`, `rollback_sync`, `start_sync_watcher`, `stop_sync_watcher` |
| **Peers** | `peer_start_discovery`, `peer_stop_discovery`, `peer_list_peers`, `peer_set_trust`, `peer_request_transfer`, `peer_respond_transfer` |
| **Server Transfer** | `server_transfer_create`, `server_transfer_start`, `server_transfer_pause`, `server_transfer_cancel`, `server_transfer_list` |
| **AI** | `ai_chat`, `ai_explain_error`, `ai_generate_suggestions`, `ai_parse_natural_language`, `ai_confirm_action`, `ai_get_audit_log`, `ai_get_feature_toggles`, `ai_set_feature_toggles` |
| **Terminal** | `terminal_create_local`, `terminal_create_remote`, `terminal_list_sessions`, `terminal_close_session`, `terminal_write`, `terminal_resize` |
| **Encryption** | `create_vault`, `unlock_vault`, `lock_vault`, `list_vaults`, `vault_encrypt_file`, `vault_decrypt_file`, `encrypt_for_upload`, `decrypt_from_download`, `change_vault_password` |
| **File Ops** | `copy_files`, `move_files`, `rename_file`, `duplicate_files`, `delete_files`, `create_directory`, `create_file`, `undo_file_operation`, `get_file_metadata` |
| **Batch Rename** | `batch_rename_preview`, `batch_rename_apply`, `batch_rename_undo` |
| **Preview** | `preview_file`, `preview_get_exif` |
| **Archive** | `archive_browse`, `archive_create`, `archive_extract`, `archive_info` |
| **Integrity** | `integrity_checksum`, `integrity_verify`, `integrity_find_duplicates`, `integrity_resolve_duplicates`, `integrity_create_tag`, `integrity_list_tags`, `integrity_tag_file`, `integrity_create_smart_folder`, `integrity_run_smart_folder` |

## Client libraries

Generated TypeScript and Python clients live in [`sdk/`](../sdk/README.md) and are served by every server at `GET /api/v1/sdk/typescript` and `GET /api/v1/sdk/python` (public, like `/api/v1/openapi.json`). They cover the JSON API under `/api/v1`, are regenerated from the OpenAPI document (`UPDATE_SDK=1 cargo test --lib sdk`), and a test fails when the committed clients drift from the API.
