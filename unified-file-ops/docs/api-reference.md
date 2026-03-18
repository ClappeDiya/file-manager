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
