# UFOP Admin Console Guide

Version 0.1.0

---

## Table of Contents

1. [Overview](#overview)
2. [Accessing the Admin Console](#accessing-the-admin-console)
3. [Dashboard](#dashboard)
4. [User Management](#user-management)
5. [Device Management](#device-management)
6. [Policies](#policies)
7. [Approval Workflows](#approval-workflows)
8. [Audit Log](#audit-log)
9. [Connectors](#connectors)
10. [Workspaces](#workspaces)
11. [AI Governance](#ai-governance)
12. [Billing](#billing)
13. [API Integration](#api-integration)

---

## Overview

The Admin Console is a Next.js web application for organizational administrators to manage users, devices, policies, connectors, and compliance for the Unified File Operations Platform.

The console is available at `http://<server>:3000` (or your configured admin URL).

---

## Accessing the Admin Console

### Authentication

Navigate to the login page. Authenticate with your admin credentials. The console supports JWT-based authentication (OAuth 2.0 client credentials for Enterprise tier) and API key authentication (Business tier).

### Roles

| Role | Permissions |
|------|-------------|
| **Super Admin** | Full access to all features, user management, policy creation |
| **Org Admin** | Organization-level management, user provisioning, policy enforcement |
| **Manager** | Team management, approval workflows, audit viewing |
| **User** | Self-service settings, view own activity |
| **Viewer** | Read-only access to dashboards and reports |

---

## Dashboard

The dashboard provides an at-a-glance overview:

### Stats Cards

- **Active Users** - Currently active users with trend from last month
- **Pending Approvals** - Number of transfer/operation approvals awaiting action
- **Online Devices** - Connected devices out of total registered, with compliance status
- **Active Policies** - Number of enforcement policies currently active

### Panels

- **Pending Approvals** - List of operations awaiting admin approval with requestor, file count, size, and timestamp
- **Recent Activity** - Latest audit log entries with severity indicators (info, warning, error, critical)
- **Device Health** - Device status list showing name, user, OS, client version, and online/degraded/error/offline status
- **Active Policies** - Current policies with enforcement mode (enforce, warn, audit)

---

## User Management

### Listing Users

The Users page (`/users`) displays all users with:

- Name and email
- Role assignment
- Active/inactive status
- Creation date
- Last activity

Use the search bar to filter by name or email. Filter by role or active status.

### Creating a User

1. Click "Add User"
2. Enter email address (required)
3. Set name and role
4. The user receives an invitation email

### Modifying Users

- Change roles
- Activate/deactivate accounts
- View user activity history

---

## Device Management

### Device Fleet

The Devices page (`/devices`) shows all registered devices:

- Device name and hostname
- Assigned user
- Operating system and client version
- Connection status (online, degraded, error, offline)
- Last seen timestamp
- Policy compliance status

### Policy Compliance

Each device displays its compliance status with active policies. Non-compliant devices are flagged with specific violations.

---

## Policies

### Policy Types

| Type | Description |
|------|-------------|
| **Transfer** | Rules for file transfers (size limits, allowed destinations, required encryption) |
| **Sync** | Rules for sync pairs (allowed directions, required verification) |
| **Storage** | Rules for file storage (naming conventions, retention periods) |
| **Security** | Rules for security (password requirements, encryption policies, TLS enforcement) |
| **Access** | Rules for access control (allowed protocols, IP restrictions) |

### Enforcement Modes

- **Enforce** - Block operations that violate the policy
- **Warn** - Allow operations but warn the user
- **Audit** - Log violations without blocking or warning

### Creating Policies

1. Navigate to Policies (`/policies`)
2. Click "Create Policy"
3. Select policy type
4. Define rules (conditions and actions)
5. Set enforcement mode
6. Assign to users, groups, or organization-wide
7. Activate the policy

### Policy Engine

The admin console includes a policy evaluation engine that checks operations against active policies. Results include:

- Pass/fail status
- Specific rule violations
- Recommended actions

---

## Approval Workflows

### How It Works

When a policy requires approval for certain operations (e.g., transfers above a size threshold, or transfers to external destinations), the operation enters a "pending" state.

### Managing Approvals

Navigate to Approvals (`/approvals`) to see:

- Pending requests with requestor, operation type, file count, and size
- Approve or deny each request
- Add notes to decisions
- View approval history

### Approval States

- **Pending** - Awaiting admin decision
- **Approved** - Operation allowed to proceed
- **Denied** - Operation blocked
- **Expired** - Timed out without a decision

---

## Audit Log

### Viewing Logs

The Audit page (`/audit`) provides a searchable, filterable log of all platform activities:

- **Event Type** - login, transfer, sync, policy change, approval, etc.
- **User** - Who performed the action
- **Timestamp** - When it occurred
- **Severity** - info, warning, error, critical
- **Description** - Human-readable summary
- **IP Address** - Source IP of the action

### Filtering

- Filter by event type, user, severity, or date range
- Full-text search on descriptions
- Pagination with configurable page size

### Audit Integrity

The admin console includes audit integrity verification to ensure log entries have not been tampered with. Each entry is cryptographically chained.

### Export

Audit logs can be exported for compliance reporting.

---

## Connectors

### Managing Connectors

The Connectors page (`/connectors`) lets admins manage organization-wide connection profiles:

- **Add** new connectors (SFTP, FTP, WebDAV, S3, GCS, Azure, SMB, NFS)
- **Test** connectivity
- **Edit** connection settings (host, port, credentials)
- **Delete** connectors
- **Connect/Disconnect** active connections

### Organization-Wide Credentials

Admins can provision shared connectors that appear for all users in the organization, with appropriate access controls.

---

## Workspaces

### Managing Workspaces

The Workspaces page (`/workspaces`) allows admins to create and manage shared workspaces:

- Create workspaces with assigned storage
- Set workspace-level policies
- Manage workspace membership
- Monitor workspace usage

---

## AI Governance

### Settings

The AI Governance page (`/ai-governance`) controls AI features across the organization:

| Setting | Description |
|---------|-------------|
| **AI Enabled** | Master switch for all AI features |
| **Auto Suggestions** | Allow AI to proactively suggest actions |
| **Natural Language Commands** | Allow users to create jobs via plain text |
| **Require Confirmation for Destructive** | Force user confirmation for AI-initiated destructive actions |
| **Max Files Per Action** | Limit the number of files an AI action can affect |
| **Allowed Actions** | Whitelist of actions AI can perform |
| **Audit Retention Days** | How long to keep AI audit logs |

### AI Audit Log

A separate audit log tracks all AI interactions:

- Input (what the user asked)
- Output (what the AI did)
- Whether the user confirmed
- Whether the action was destructive
- Timestamps and metadata

### Feature Toggles

Enable or disable specific AI capabilities per organization.

---

## Billing

The Billing page (`/billing`) displays:

- Current plan and tier
- Usage statistics
- Payment history
- Plan upgrade options

---

## API Integration

### OpenAPI Specification

The full API specification is available at `GET /api/openapi` as an OpenAPI 3.0 JSON document.

### Authentication

API requests require either:

- **Bearer Token** - JWT from OAuth 2.0 client credentials flow
- **API Key** - Passed in the `X-API-Key` header

### Webhooks

Register webhooks to receive real-time notifications:

1. Navigate to the Webhooks section (via API)
2. Register a URL and select events to subscribe to
3. Available events include transfer completion, sync status changes, policy violations, and approval requests

### Rate Limiting

API endpoints are rate-limited. A `429 Too Many Requests` response indicates the limit has been exceeded.

See [API Reference](./api-reference.md) for full endpoint documentation.
