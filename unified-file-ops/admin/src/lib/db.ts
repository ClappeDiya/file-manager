/**
 * SQLite database layer for the UFOP admin console.
 *
 * Replaces the in-memory DataStore with persistent SQLite storage.
 * Database file: admin/.data/admin.db (auto-created, gitignored)
 *
 * All complex/nested fields are stored as JSON TEXT columns.
 */

import Database from 'better-sqlite3';
import path from 'path';
import fs from 'fs';
import type { AdminUser } from './types/auth';
import type { PolicyRule } from './types/policies';
import type { ApprovalRequest } from './types/approvals';
import type { AuditEntry, DeviceHealth } from './types/audit';
import type { Workspace, SharedConnection, SyncTemplate } from './types/workspaces';

// ── Database path ──

const DATA_DIR = path.join(process.cwd(), '.data');
const DB_PATH = process.env.ADMIN_DB_PATH || path.join(DATA_DIR, 'admin.db');

// ── Lazy singleton ──

let _db: Database.Database | null = null;

function getDb(): Database.Database {
  if (_db) return _db;

  fs.mkdirSync(path.dirname(DB_PATH), { recursive: true });
  _db = new Database(DB_PATH);
  _db.pragma('journal_mode = WAL');
  _db.pragma('foreign_keys = ON');
  initSchema(_db);

  // Seed if empty
  const count = _db.prepare('SELECT COUNT(*) as c FROM users').get() as { c: number };
  if (count.c === 0) {
    // Dynamic import to avoid circular dependency
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const { seedDatabase } = require('./db-seed');
    seedDatabase(_db);
  }

  return _db;
}

// ── Schema ──

function initSchema(db: Database.Database) {
  db.exec(`
    CREATE TABLE IF NOT EXISTS users (
      id TEXT PRIMARY KEY,
      email TEXT UNIQUE NOT NULL,
      name TEXT NOT NULL,
      role TEXT NOT NULL DEFAULT 'user',
      permissions TEXT DEFAULT '[]',
      organization_id TEXT NOT NULL,
      last_login_at TEXT,
      created_at TEXT NOT NULL,
      is_active INTEGER NOT NULL DEFAULT 1
    );

    CREATE TABLE IF NOT EXISTS policies (
      id TEXT PRIMARY KEY,
      domain TEXT NOT NULL,
      name TEXT NOT NULL,
      description TEXT DEFAULT '',
      assignment_target TEXT NOT NULL,
      assignment_value TEXT NOT NULL,
      enforcement_mode TEXT NOT NULL DEFAULT 'enforce',
      configuration TEXT DEFAULT '{}',
      created_by TEXT NOT NULL,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL,
      version INTEGER NOT NULL DEFAULT 1,
      is_active INTEGER NOT NULL DEFAULT 1
    );

    CREATE TABLE IF NOT EXISTS approvals (
      id TEXT PRIMARY KEY,
      operation TEXT NOT NULL,
      trigger_type TEXT NOT NULL,
      source TEXT NOT NULL,
      destination TEXT NOT NULL,
      file_count INTEGER DEFAULT 0,
      total_size INTEGER DEFAULT 0,
      reason TEXT DEFAULT '',
      triggering_policy TEXT DEFAULT '',
      state TEXT NOT NULL DEFAULT 'pending',
      requested_by TEXT NOT NULL,
      requested_by_name TEXT NOT NULL,
      requested_at TEXT NOT NULL,
      expires_at TEXT NOT NULL,
      reviewed_by TEXT,
      reviewed_by_name TEXT,
      reviewed_at TEXT,
      review_comment TEXT,
      organization_id TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS audit_entries (
      id TEXT PRIMARY KEY,
      event_type TEXT NOT NULL,
      severity TEXT NOT NULL,
      timestamp TEXT NOT NULL,
      user_id TEXT NOT NULL,
      user_name TEXT NOT NULL,
      user_email TEXT NOT NULL,
      organization_id TEXT NOT NULL,
      ip_address TEXT,
      user_agent TEXT,
      description TEXT NOT NULL,
      metadata TEXT DEFAULT '{}',
      resource_type TEXT,
      resource_id TEXT,
      integrity_hash TEXT,
      previous_hash TEXT
    );

    CREATE TABLE IF NOT EXISTS devices (
      id TEXT PRIMARY KEY,
      device_id TEXT NOT NULL,
      device_name TEXT NOT NULL,
      user_id TEXT NOT NULL,
      user_name TEXT NOT NULL,
      client_version TEXT NOT NULL,
      os TEXT NOT NULL,
      os_version TEXT NOT NULL,
      last_seen_at TEXT NOT NULL,
      status TEXT NOT NULL DEFAULT 'online',
      failure_patterns TEXT DEFAULT '[]',
      policy_compliance TEXT DEFAULT '{}',
      cpu_usage REAL DEFAULT 0,
      memory_usage REAL DEFAULT 0,
      disk_usage REAL DEFAULT 0
    );

    CREATE TABLE IF NOT EXISTS workspaces (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      description TEXT DEFAULT '',
      organization_id TEXT NOT NULL,
      connections TEXT DEFAULT '[]',
      sync_templates TEXT DEFAULT '[]',
      automation_templates TEXT DEFAULT '[]',
      members TEXT DEFAULT '[]',
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS shared_connections (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      connector_type TEXT NOT NULL,
      config TEXT DEFAULT '{}',
      owner_id TEXT NOT NULL,
      owner_name TEXT NOT NULL,
      organization_id TEXT NOT NULL,
      shared_with TEXT DEFAULT '[]',
      last_propagated_at TEXT,
      propagation_status TEXT DEFAULT '{}',
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL,
      is_active INTEGER NOT NULL DEFAULT 1
    );

    CREATE TABLE IF NOT EXISTS sync_templates (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      description TEXT DEFAULT '',
      source_connector TEXT NOT NULL,
      source_path TEXT NOT NULL,
      dest_connector TEXT NOT NULL,
      dest_path TEXT NOT NULL,
      sync_mode TEXT NOT NULL,
      conflict_resolution TEXT NOT NULL,
      file_filters TEXT DEFAULT '[]',
      exclude_patterns TEXT DEFAULT '[]',
      schedule TEXT,
      organization_id TEXT NOT NULL,
      owner_id TEXT NOT NULL,
      owner_name TEXT NOT NULL,
      shared_with TEXT DEFAULT '[]',
      activated_by_users TEXT DEFAULT '[]',
      propagation_status TEXT DEFAULT '{}',
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL,
      is_active INTEGER NOT NULL DEFAULT 1
    );

    CREATE TABLE IF NOT EXISTS connectors (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      type TEXT NOT NULL,
      status TEXT NOT NULL DEFAULT 'active',
      endpoint TEXT,
      region TEXT,
      created_at TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS ai_governance (
      key TEXT PRIMARY KEY,
      value TEXT NOT NULL
    );
  `);
}

// ── Row to Type converters ──

function rowToUser(row: Record<string, unknown>): AdminUser {
  return {
    id: row.id as string,
    email: row.email as string,
    name: row.name as string,
    role: row.role as AdminUser['role'],
    permissions: JSON.parse((row.permissions as string) || '[]'),
    organizationId: row.organization_id as string,
    lastLoginAt: (row.last_login_at as string) || undefined,
    createdAt: row.created_at as string,
    isActive: row.is_active === 1,
  };
}

function rowToPolicy(row: Record<string, unknown>): PolicyRule {
  return {
    id: row.id as string,
    domain: row.domain as PolicyRule['domain'],
    name: row.name as string,
    description: row.description as string,
    assignmentTarget: row.assignment_target as PolicyRule['assignmentTarget'],
    assignmentValue: row.assignment_value as string,
    enforcementMode: row.enforcement_mode as PolicyRule['enforcementMode'],
    configuration: JSON.parse((row.configuration as string) || '{}'),
    createdBy: row.created_by as string,
    createdAt: row.created_at as string,
    updatedAt: row.updated_at as string,
    version: row.version as number,
    isActive: row.is_active === 1,
  };
}

function rowToApproval(row: Record<string, unknown>): ApprovalRequest {
  return {
    id: row.id as string,
    operation: row.operation as string,
    trigger: row.trigger_type as ApprovalRequest['trigger'],
    source: row.source as string,
    destination: row.destination as string,
    fileCount: row.file_count as number,
    totalSize: row.total_size as number,
    reason: row.reason as string,
    triggeringPolicy: row.triggering_policy as string,
    state: row.state as ApprovalRequest['state'],
    requestedBy: row.requested_by as string,
    requestedByName: row.requested_by_name as string,
    requestedAt: row.requested_at as string,
    expiresAt: row.expires_at as string,
    reviewedBy: (row.reviewed_by as string) || undefined,
    reviewedByName: (row.reviewed_by_name as string) || undefined,
    reviewedAt: (row.reviewed_at as string) || undefined,
    reviewComment: (row.review_comment as string) || undefined,
    organizationId: row.organization_id as string,
  };
}

function rowToAuditEntry(row: Record<string, unknown>): AuditEntry {
  return {
    id: row.id as string,
    eventType: row.event_type as AuditEntry['eventType'],
    severity: row.severity as AuditEntry['severity'],
    timestamp: row.timestamp as string,
    userId: row.user_id as string,
    userName: row.user_name as string,
    userEmail: row.user_email as string,
    organizationId: row.organization_id as string,
    ipAddress: (row.ip_address as string) || '',
    userAgent: (row.user_agent as string) || '',
    description: row.description as string,
    metadata: JSON.parse((row.metadata as string) || '{}'),
    resourceType: (row.resource_type as string) || undefined,
    resourceId: (row.resource_id as string) || undefined,
    integrityHash: (row.integrity_hash as string) || '',
    previousHash: (row.previous_hash as string) || undefined,
  };
}

function rowToDevice(row: Record<string, unknown>): DeviceHealth {
  return {
    id: row.id as string,
    deviceId: row.device_id as string,
    deviceName: row.device_name as string,
    userId: row.user_id as string,
    userName: row.user_name as string,
    clientVersion: row.client_version as string,
    os: row.os as string,
    osVersion: row.os_version as string,
    lastSeenAt: row.last_seen_at as string,
    status: row.status as DeviceHealth['status'],
    failurePatterns: JSON.parse((row.failure_patterns as string) || '[]'),
    policyCompliance: JSON.parse((row.policy_compliance as string) || '{}'),
    cpuUsage: row.cpu_usage as number,
    memoryUsage: row.memory_usage as number,
    diskUsage: row.disk_usage as number,
  };
}

function rowToWorkspace(row: Record<string, unknown>): Workspace {
  return {
    id: row.id as string,
    name: row.name as string,
    description: (row.description as string) || '',
    organizationId: row.organization_id as string,
    connections: JSON.parse((row.connections as string) || '[]'),
    syncTemplates: JSON.parse((row.sync_templates as string) || '[]'),
    automationTemplates: JSON.parse((row.automation_templates as string) || '[]'),
    members: JSON.parse((row.members as string) || '[]'),
    createdAt: row.created_at as string,
    updatedAt: row.updated_at as string,
  };
}

function rowToSharedConnection(row: Record<string, unknown>): SharedConnection {
  return {
    id: row.id as string,
    name: row.name as string,
    connectorType: row.connector_type as string,
    config: JSON.parse((row.config as string) || '{}'),
    ownerId: row.owner_id as string,
    ownerName: row.owner_name as string,
    organizationId: row.organization_id as string,
    sharedWith: JSON.parse((row.shared_with as string) || '[]'),
    lastPropagatedAt: (row.last_propagated_at as string) || undefined,
    propagationStatus: JSON.parse((row.propagation_status as string) || '{}'),
    createdAt: row.created_at as string,
    updatedAt: row.updated_at as string,
    isActive: row.is_active === 1,
  };
}

function rowToSyncTemplate(row: Record<string, unknown>): SyncTemplate {
  return {
    id: row.id as string,
    name: row.name as string,
    description: (row.description as string) || '',
    sourceConnector: row.source_connector as string,
    sourcePath: row.source_path as string,
    destConnector: row.dest_connector as string,
    destPath: row.dest_path as string,
    syncMode: row.sync_mode as SyncTemplate['syncMode'],
    conflictResolution: row.conflict_resolution as SyncTemplate['conflictResolution'],
    fileFilters: JSON.parse((row.file_filters as string) || '[]'),
    excludePatterns: JSON.parse((row.exclude_patterns as string) || '[]'),
    schedule: (row.schedule as string) || undefined,
    organizationId: row.organization_id as string,
    ownerId: row.owner_id as string,
    ownerName: row.owner_name as string,
    sharedWith: JSON.parse((row.shared_with as string) || '[]'),
    activatedByUsers: JSON.parse((row.activated_by_users as string) || '[]'),
    propagationStatus: JSON.parse((row.propagation_status as string) || '{}'),
    createdAt: row.created_at as string,
    updatedAt: row.updated_at as string,
    isActive: row.is_active === 1,
  };
}

// ── CRUD: Users ──

export function getUsers(): AdminUser[] {
  return (getDb().prepare('SELECT * FROM users').all() as Record<string, unknown>[]).map(rowToUser);
}

export function getUserById(id: string): AdminUser | undefined {
  const row = getDb().prepare('SELECT * FROM users WHERE id = ?').get(id) as Record<string, unknown> | undefined;
  return row ? rowToUser(row) : undefined;
}

export function getUserByEmail(email: string): AdminUser | undefined {
  const row = getDb().prepare('SELECT * FROM users WHERE email = ?').get(email) as Record<string, unknown> | undefined;
  return row ? rowToUser(row) : undefined;
}

export function createUser(user: AdminUser): void {
  getDb().prepare(`
    INSERT INTO users (id, email, name, role, permissions, organization_id, last_login_at, created_at, is_active)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
  `).run(user.id, user.email, user.name, user.role, JSON.stringify(user.permissions), user.organizationId, user.lastLoginAt || null, user.createdAt, user.isActive ? 1 : 0);
}

export function updateUser(id: string, updates: Partial<AdminUser>): void {
  const user = getUserById(id);
  if (!user) return;
  const merged = { ...user, ...updates };
  getDb().prepare(`
    UPDATE users SET email = ?, name = ?, role = ?, permissions = ?, organization_id = ?, last_login_at = ?, is_active = ?
    WHERE id = ?
  `).run(merged.email, merged.name, merged.role, JSON.stringify(merged.permissions), merged.organizationId, merged.lastLoginAt || null, merged.isActive ? 1 : 0, id);
}

export function deleteUser(id: string): void {
  getDb().prepare('DELETE FROM users WHERE id = ?').run(id);
}

export function deleteUsers(ids: string[]): void {
  const placeholders = ids.map(() => '?').join(',');
  getDb().prepare(`DELETE FROM users WHERE id IN (${placeholders})`).run(...ids);
}

export function updateUsers(ids: string[], updates: Partial<AdminUser>): void {
  const stmt = getDb().prepare('UPDATE users SET is_active = ? WHERE id = ?');
  const tx = getDb().transaction(() => {
    for (const id of ids) {
      if (updates.isActive !== undefined) {
        stmt.run(updates.isActive ? 1 : 0, id);
      }
    }
  });
  tx();
}

// ── CRUD: Policies ──

export function getPolicies(): PolicyRule[] {
  return (getDb().prepare('SELECT * FROM policies').all() as Record<string, unknown>[]).map(rowToPolicy);
}

export function getPolicyById(id: string): PolicyRule | undefined {
  const row = getDb().prepare('SELECT * FROM policies WHERE id = ?').get(id) as Record<string, unknown> | undefined;
  return row ? rowToPolicy(row) : undefined;
}

export function createPolicy(policy: PolicyRule): void {
  getDb().prepare(`
    INSERT INTO policies (id, domain, name, description, assignment_target, assignment_value, enforcement_mode, configuration, created_by, created_at, updated_at, version, is_active)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
  `).run(policy.id, policy.domain, policy.name, policy.description, policy.assignmentTarget, policy.assignmentValue, policy.enforcementMode, JSON.stringify(policy.configuration), policy.createdBy, policy.createdAt, policy.updatedAt, policy.version, policy.isActive ? 1 : 0);
}

export function updatePolicy(id: string, updates: Partial<PolicyRule>): PolicyRule | undefined {
  const existing = getPolicyById(id);
  if (!existing) return undefined;
  const merged = { ...existing, ...updates, updatedAt: new Date().toISOString(), version: existing.version + 1 };
  getDb().prepare(`
    UPDATE policies SET domain = ?, name = ?, description = ?, assignment_target = ?, assignment_value = ?,
      enforcement_mode = ?, configuration = ?, updated_at = ?, version = ?, is_active = ?
    WHERE id = ?
  `).run(merged.domain, merged.name, merged.description, merged.assignmentTarget, merged.assignmentValue, merged.enforcementMode, JSON.stringify(merged.configuration), merged.updatedAt, merged.version, merged.isActive ? 1 : 0, id);
  return merged;
}

export function deletePolicy(id: string): void {
  getDb().prepare('DELETE FROM policies WHERE id = ?').run(id);
}

// ── CRUD: Approvals ──

export function getApprovals(): ApprovalRequest[] {
  return (getDb().prepare('SELECT * FROM approvals').all() as Record<string, unknown>[]).map(rowToApproval);
}

export function createApproval(approval: ApprovalRequest): void {
  getDb().prepare(`
    INSERT INTO approvals (id, operation, trigger_type, source, destination, file_count, total_size, reason, triggering_policy, state, requested_by, requested_by_name, requested_at, expires_at, reviewed_by, reviewed_by_name, reviewed_at, review_comment, organization_id)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
  `).run(approval.id, approval.operation, approval.trigger, approval.source, approval.destination, approval.fileCount, approval.totalSize, approval.reason, approval.triggeringPolicy, approval.state, approval.requestedBy, approval.requestedByName, approval.requestedAt, approval.expiresAt, approval.reviewedBy || null, approval.reviewedByName || null, approval.reviewedAt || null, approval.reviewComment || null, approval.organizationId);
}

export function updateApproval(id: string, updates: Partial<ApprovalRequest>): void {
  const existing = getApprovals().find(a => a.id === id);
  if (!existing) return;
  const merged = { ...existing, ...updates };
  getDb().prepare(`
    UPDATE approvals SET state = ?, reviewed_by = ?, reviewed_by_name = ?, reviewed_at = ?, review_comment = ?
    WHERE id = ?
  `).run(merged.state, merged.reviewedBy || null, merged.reviewedByName || null, merged.reviewedAt || null, merged.reviewComment || null, id);
}

export function expireApprovals(): number {
  const now = new Date().toISOString();
  const result = getDb().prepare(`UPDATE approvals SET state = 'expired' WHERE state = 'pending' AND expires_at < ?`).run(now);
  return result.changes;
}

// ── CRUD: Audit Entries ──

export function getAuditEntries(): AuditEntry[] {
  return (getDb().prepare('SELECT * FROM audit_entries ORDER BY timestamp DESC').all() as Record<string, unknown>[]).map(rowToAuditEntry);
}

export function createAuditEntry(entry: AuditEntry): void {
  getDb().prepare(`
    INSERT INTO audit_entries (id, event_type, severity, timestamp, user_id, user_name, user_email, organization_id, ip_address, user_agent, description, metadata, resource_type, resource_id, integrity_hash, previous_hash)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
  `).run(entry.id, entry.eventType, entry.severity, entry.timestamp, entry.userId, entry.userName, entry.userEmail, entry.organizationId, entry.ipAddress || null, entry.userAgent || null, entry.description, JSON.stringify(entry.metadata || {}), entry.resourceType || null, entry.resourceId || null, entry.integrityHash || null, entry.previousHash || null);
}

// ── CRUD: Devices ──

export function getDevices(): DeviceHealth[] {
  return (getDb().prepare('SELECT * FROM devices').all() as Record<string, unknown>[]).map(rowToDevice);
}

export function createDevice(device: DeviceHealth): void {
  getDb().prepare(`
    INSERT INTO devices (id, device_id, device_name, user_id, user_name, client_version, os, os_version, last_seen_at, status, failure_patterns, policy_compliance, cpu_usage, memory_usage, disk_usage)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
  `).run(device.id, device.deviceId, device.deviceName, device.userId, device.userName, device.clientVersion, device.os, device.osVersion, device.lastSeenAt, device.status, JSON.stringify(device.failurePatterns), JSON.stringify(device.policyCompliance), device.cpuUsage, device.memoryUsage, device.diskUsage);
}

// ── CRUD: Workspaces ──

export function getWorkspaces(): Workspace[] {
  return (getDb().prepare('SELECT * FROM workspaces').all() as Record<string, unknown>[]).map(rowToWorkspace);
}

export function createWorkspace(ws: Workspace): void {
  getDb().prepare(`
    INSERT INTO workspaces (id, name, description, organization_id, connections, sync_templates, automation_templates, members, created_at, updated_at)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
  `).run(ws.id, ws.name, ws.description, ws.organizationId, JSON.stringify(ws.connections), JSON.stringify(ws.syncTemplates), JSON.stringify(ws.automationTemplates), JSON.stringify(ws.members), ws.createdAt, ws.updatedAt);
}

// ── CRUD: Shared Connections ──

export function getSharedConnections(): SharedConnection[] {
  return (getDb().prepare('SELECT * FROM shared_connections').all() as Record<string, unknown>[]).map(rowToSharedConnection);
}

export function createSharedConnection(conn: SharedConnection): void {
  getDb().prepare(`
    INSERT INTO shared_connections (id, name, connector_type, config, owner_id, owner_name, organization_id, shared_with, last_propagated_at, propagation_status, created_at, updated_at, is_active)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
  `).run(conn.id, conn.name, conn.connectorType, JSON.stringify(conn.config), conn.ownerId, conn.ownerName, conn.organizationId, JSON.stringify(conn.sharedWith), conn.lastPropagatedAt || null, JSON.stringify(conn.propagationStatus), conn.createdAt, conn.updatedAt, conn.isActive ? 1 : 0);
}

// ── CRUD: Sync Templates ──

export function getSyncTemplates(): SyncTemplate[] {
  return (getDb().prepare('SELECT * FROM sync_templates').all() as Record<string, unknown>[]).map(rowToSyncTemplate);
}

export function createSyncTemplate(tmpl: SyncTemplate): void {
  getDb().prepare(`
    INSERT INTO sync_templates (id, name, description, source_connector, source_path, dest_connector, dest_path, sync_mode, conflict_resolution, file_filters, exclude_patterns, schedule, organization_id, owner_id, owner_name, shared_with, activated_by_users, propagation_status, created_at, updated_at, is_active)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
  `).run(tmpl.id, tmpl.name, tmpl.description, tmpl.sourceConnector, tmpl.sourcePath, tmpl.destConnector, tmpl.destPath, tmpl.syncMode, tmpl.conflictResolution, JSON.stringify(tmpl.fileFilters), JSON.stringify(tmpl.excludePatterns), tmpl.schedule || null, tmpl.organizationId, tmpl.ownerId, tmpl.ownerName, JSON.stringify(tmpl.sharedWith), JSON.stringify(tmpl.activatedByUsers), JSON.stringify(tmpl.propagationStatus), tmpl.createdAt, tmpl.updatedAt, tmpl.isActive ? 1 : 0);
}

// ── CRUD: Connectors ──

export interface Connector {
  id: string;
  name: string;
  type: string;
  status: string;
  endpoint?: string;
  region?: string;
  createdAt: string;
}

export function getConnectors(): Connector[] {
  return (getDb().prepare('SELECT * FROM connectors').all() as Record<string, unknown>[]).map(row => ({
    id: row.id as string,
    name: row.name as string,
    type: row.type as string,
    status: row.status as string,
    endpoint: (row.endpoint as string) || undefined,
    region: (row.region as string) || undefined,
    createdAt: row.created_at as string,
  }));
}

export function createConnector(c: Connector): void {
  getDb().prepare(`
    INSERT INTO connectors (id, name, type, status, endpoint, region, created_at)
    VALUES (?, ?, ?, ?, ?, ?, ?)
  `).run(c.id, c.name, c.type, c.status, c.endpoint || null, c.region || null, c.createdAt);
}

export function updateConnector(id: string, updates: Partial<Connector>): void {
  const existing = getConnectors().find(c => c.id === id);
  if (!existing) return;
  const merged = { ...existing, ...updates };
  getDb().prepare('UPDATE connectors SET name = ?, type = ?, status = ?, endpoint = ?, region = ? WHERE id = ?')
    .run(merged.name, merged.type, merged.status, merged.endpoint || null, merged.region || null, id);
}

export function deleteConnector(id: string): void {
  getDb().prepare('DELETE FROM connectors WHERE id = ?').run(id);
}

// ── CRUD: AI Governance ──

export function getAiGovernanceSettings(): Record<string, unknown> {
  const rows = getDb().prepare('SELECT * FROM ai_governance').all() as { key: string; value: string }[];
  const result: Record<string, unknown> = {};
  for (const row of rows) {
    try { result[row.key] = JSON.parse(row.value); } catch { result[row.key] = row.value; }
  }
  return result;
}

export function setAiGovernanceSetting(key: string, value: unknown): void {
  getDb().prepare('INSERT OR REPLACE INTO ai_governance (key, value) VALUES (?, ?)').run(key, JSON.stringify(value));
}
