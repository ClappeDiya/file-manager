/**
 * Database seeding module.
 *
 * Populates the SQLite database with initial data from mock-data.ts
 * on first boot (when tables are empty). This runs once and never again
 * unless the database file is deleted.
 */

import type Database from 'better-sqlite3';
import { mockUsers, mockPolicies, mockApprovals, mockAuditEntries, mockDevices, mockWorkspaces, mockSharedConnections, mockSyncTemplates } from './mock-data';

export function seedDatabase(db: Database.Database): void {
  console.log('[db-seed] Seeding database with initial data...');

  const tx = db.transaction(() => {
    // Users
    const insertUser = db.prepare(`
      INSERT INTO users (id, email, name, role, permissions, organization_id, last_login_at, created_at, is_active)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
    `);
    for (const u of mockUsers) {
      insertUser.run(u.id, u.email, u.name, u.role, JSON.stringify(u.permissions), u.organizationId, u.lastLoginAt || null, u.createdAt, u.isActive ? 1 : 0);
    }

    // Policies
    const insertPolicy = db.prepare(`
      INSERT INTO policies (id, domain, name, description, assignment_target, assignment_value, enforcement_mode, configuration, created_by, created_at, updated_at, version, is_active)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    `);
    for (const p of mockPolicies) {
      insertPolicy.run(p.id, p.domain, p.name, p.description, p.assignmentTarget, p.assignmentValue, p.enforcementMode, JSON.stringify(p.configuration), p.createdBy, p.createdAt, p.updatedAt, p.version, p.isActive ? 1 : 0);
    }

    // Approvals
    const insertApproval = db.prepare(`
      INSERT INTO approvals (id, operation, trigger_type, source, destination, file_count, total_size, reason, triggering_policy, state, requested_by, requested_by_name, requested_at, expires_at, reviewed_by, reviewed_by_name, reviewed_at, review_comment, organization_id)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    `);
    for (const a of mockApprovals) {
      insertApproval.run(a.id, a.operation, a.trigger, a.source, a.destination, a.fileCount, a.totalSize, a.reason, a.triggeringPolicy, a.state, a.requestedBy, a.requestedByName, a.requestedAt, a.expiresAt, a.reviewedBy || null, a.reviewedByName || null, a.reviewedAt || null, a.reviewComment || null, a.organizationId);
    }

    // Audit entries
    const insertAudit = db.prepare(`
      INSERT INTO audit_entries (id, event_type, severity, timestamp, user_id, user_name, user_email, organization_id, ip_address, user_agent, description, metadata, resource_type, resource_id, integrity_hash, previous_hash)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    `);
    for (const e of mockAuditEntries) {
      insertAudit.run(e.id, e.eventType, e.severity, e.timestamp, e.userId, e.userName, e.userEmail, e.organizationId, e.ipAddress || null, e.userAgent || null, e.description, JSON.stringify(e.metadata || {}), e.resourceType || null, e.resourceId || null, e.integrityHash || null, e.previousHash || null);
    }

    // Devices
    const insertDevice = db.prepare(`
      INSERT INTO devices (id, device_id, device_name, user_id, user_name, client_version, os, os_version, last_seen_at, status, failure_patterns, policy_compliance, cpu_usage, memory_usage, disk_usage)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    `);
    for (const d of mockDevices) {
      insertDevice.run(d.id, d.deviceId, d.deviceName, d.userId, d.userName, d.clientVersion, d.os, d.osVersion, d.lastSeenAt, d.status, JSON.stringify(d.failurePatterns), JSON.stringify(d.policyCompliance), d.cpuUsage, d.memoryUsage, d.diskUsage);
    }

    // Workspaces
    const insertWorkspace = db.prepare(`
      INSERT INTO workspaces (id, name, description, organization_id, connections, sync_templates, automation_templates, members, created_at, updated_at)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    `);
    for (const ws of mockWorkspaces) {
      insertWorkspace.run(ws.id, ws.name, ws.description, ws.organizationId, JSON.stringify(ws.connections), JSON.stringify(ws.syncTemplates), JSON.stringify(ws.automationTemplates), JSON.stringify(ws.members), ws.createdAt, ws.updatedAt);
    }

    // Shared connections
    const insertSharedConn = db.prepare(`
      INSERT INTO shared_connections (id, name, connector_type, config, owner_id, owner_name, organization_id, shared_with, last_propagated_at, propagation_status, created_at, updated_at, is_active)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    `);
    for (const c of mockSharedConnections) {
      insertSharedConn.run(c.id, c.name, c.connectorType, JSON.stringify(c.config), c.ownerId, c.ownerName, c.organizationId, JSON.stringify(c.sharedWith), c.lastPropagatedAt || null, JSON.stringify(c.propagationStatus), c.createdAt, c.updatedAt, c.isActive ? 1 : 0);
    }

    // Sync templates
    const insertSyncTmpl = db.prepare(`
      INSERT INTO sync_templates (id, name, description, source_connector, source_path, dest_connector, dest_path, sync_mode, conflict_resolution, file_filters, exclude_patterns, schedule, organization_id, owner_id, owner_name, shared_with, activated_by_users, propagation_status, created_at, updated_at, is_active)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    `);
    for (const t of mockSyncTemplates) {
      insertSyncTmpl.run(t.id, t.name, t.description, t.sourceConnector, t.sourcePath, t.destConnector, t.destPath, t.syncMode, t.conflictResolution, JSON.stringify(t.fileFilters), JSON.stringify(t.excludePatterns), t.schedule || null, t.organizationId, t.ownerId, t.ownerName, JSON.stringify(t.sharedWith), JSON.stringify(t.activatedByUsers), JSON.stringify(t.propagationStatus), t.createdAt, t.updatedAt, t.isActive ? 1 : 0);
    }

    // Seed default connectors
    const insertConnector = db.prepare(`
      INSERT INTO connectors (id, name, type, status, endpoint, region, created_at)
      VALUES (?, ?, ?, ?, ?, ?, ?)
    `);
    insertConnector.run('conn_001', 'Production S3', 's3', 'active', 's3.amazonaws.com', 'us-east-1', '2025-06-01T10:00:00Z');
    insertConnector.run('conn_002', 'Azure Blob Storage', 'azure_blob', 'active', 'acme.blob.core.windows.net', 'eastus', '2025-06-15T10:00:00Z');
    insertConnector.run('conn_003', 'Backup SFTP', 'sftp', 'active', 'sftp.acme.com', null, '2025-07-01T10:00:00Z');

    // Seed default AI governance settings
    const insertAiGov = db.prepare('INSERT INTO ai_governance (key, value) VALUES (?, ?)');
    insertAiGov.run('fileClassification', JSON.stringify(true));
    insertAiGov.run('contentAnalysis', JSON.stringify(false));
    insertAiGov.run('namingSuggestions', JSON.stringify(true));
    insertAiGov.run('anomalyDetection', JSON.stringify(true));
    insertAiGov.run('dataResidency', JSON.stringify('local'));
    insertAiGov.run('excludedPaths', JSON.stringify(['/confidential', '/legal', '/hr']));
  });

  tx();
  console.log('[db-seed] Database seeded successfully.');
}
