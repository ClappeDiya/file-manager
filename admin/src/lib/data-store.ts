/**
 * SQLite-backed data store for the admin console.
 *
 * Uses better-sqlite3 for persistent data storage. The database file
 * is stored at ADMIN_DB_PATH (default: ./admin-data.db).
 *
 * All data persists across server restarts. Tables are auto-created
 * on first access via migrations.
 */

import Database from 'better-sqlite3';
import path from 'path';
import type { AdminUser, Permission } from './types/auth';
import type { PolicyRule } from './types/policies';
import type { ApprovalRequest } from './types/approvals';
import type { AuditEntry, DeviceHealth } from './types/audit';
import type { SharedConnection, SyncTemplate, Workspace } from './types/workspaces';

const DB_PATH = process.env.ADMIN_DB_PATH || path.join(process.cwd(), 'admin-data.db');

/** Initialize the database with required tables. */
function initDatabase(db: Database.Database) {
  db.pragma('journal_mode = WAL');
  db.pragma('foreign_keys = ON');

  db.exec(`
    CREATE TABLE IF NOT EXISTS users (
      id TEXT PRIMARY KEY,
      email TEXT UNIQUE NOT NULL,
      name TEXT NOT NULL DEFAULT '',
      role TEXT NOT NULL DEFAULT 'user',
      password_hash TEXT,
      permissions TEXT DEFAULT '[]',
      organization_id TEXT DEFAULT '',
      last_login_at TEXT,
      created_at TEXT NOT NULL DEFAULT (datetime('now')),
      is_active INTEGER NOT NULL DEFAULT 1,
      sso_provider TEXT,
      mfa_enabled INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE IF NOT EXISTS policies (
      id TEXT PRIMARY KEY,
      domain TEXT NOT NULL,
      name TEXT NOT NULL,
      assignment_target TEXT NOT NULL DEFAULT 'organization',
      assignment_value TEXT DEFAULT '',
      enforcement_mode TEXT NOT NULL DEFAULT 'enforce',
      configuration TEXT NOT NULL DEFAULT '{}',
      created_by TEXT DEFAULT '',
      version INTEGER NOT NULL DEFAULT 1,
      is_active INTEGER NOT NULL DEFAULT 1,
      created_at TEXT NOT NULL DEFAULT (datetime('now')),
      updated_at TEXT NOT NULL DEFAULT (datetime('now'))
    );

    CREATE TABLE IF NOT EXISTS approvals (
      id TEXT PRIMARY KEY,
      operation TEXT NOT NULL,
      trigger_type TEXT DEFAULT '',
      source TEXT DEFAULT '',
      destination TEXT DEFAULT '',
      file_count INTEGER DEFAULT 0,
      total_size INTEGER DEFAULT 0,
      state TEXT NOT NULL DEFAULT 'pending',
      requested_by TEXT DEFAULT '',
      requested_at TEXT NOT NULL DEFAULT (datetime('now')),
      reviewed_by TEXT,
      reviewed_at TEXT,
      review_comment TEXT,
      expires_at TEXT
    );

    CREATE TABLE IF NOT EXISTS audit_entries (
      id TEXT PRIMARY KEY,
      event_type TEXT NOT NULL,
      severity TEXT NOT NULL DEFAULT 'info',
      timestamp TEXT NOT NULL DEFAULT (datetime('now')),
      user_id TEXT DEFAULT '',
      user_email TEXT DEFAULT '',
      ip_address TEXT DEFAULT '',
      user_agent TEXT DEFAULT '',
      description TEXT NOT NULL DEFAULT '',
      metadata TEXT DEFAULT '{}',
      integrity_hash TEXT DEFAULT '',
      previous_hash TEXT DEFAULT ''
    );

    CREATE TABLE IF NOT EXISTS devices (
      id TEXT PRIMARY KEY,
      device_id TEXT UNIQUE NOT NULL,
      device_name TEXT NOT NULL DEFAULT '',
      user_id TEXT DEFAULT '',
      user_email TEXT DEFAULT '',
      client_version TEXT DEFAULT '',
      os TEXT DEFAULT '',
      arch TEXT DEFAULT '',
      status TEXT NOT NULL DEFAULT 'offline',
      last_seen TEXT,
      ip_address TEXT DEFAULT '',
      failure_patterns TEXT DEFAULT '[]',
      policy_compliance TEXT DEFAULT '{}',
      cpu_usage REAL DEFAULT 0,
      memory_usage REAL DEFAULT 0,
      disk_usage REAL DEFAULT 0
    );

    CREATE TABLE IF NOT EXISTS workspaces (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      organization_id TEXT DEFAULT '',
      connections TEXT DEFAULT '[]',
      sync_templates TEXT DEFAULT '[]',
      automation_templates TEXT DEFAULT '[]',
      members TEXT DEFAULT '[]',
      created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );

    CREATE TABLE IF NOT EXISTS shared_connections (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      connector_type TEXT NOT NULL DEFAULT '',
      config TEXT DEFAULT '{}',
      owner_id TEXT DEFAULT '',
      shared_with TEXT DEFAULT '[]',
      propagation_status TEXT DEFAULT 'active',
      created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );

    CREATE TABLE IF NOT EXISTS sync_templates (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      source_connector TEXT DEFAULT '',
      source_path TEXT DEFAULT '',
      dest_connector TEXT DEFAULT '',
      dest_path TEXT DEFAULT '',
      sync_mode TEXT DEFAULT 'one_way',
      conflict_resolution TEXT DEFAULT 'newest_wins',
      schedule TEXT DEFAULT '',
      activated_by_users TEXT DEFAULT '[]',
      propagation_status TEXT DEFAULT 'active',
      created_at TEXT NOT NULL DEFAULT (datetime('now'))
    );
  `);
}

/** Parse JSON column with fallback */
function parseJson<T>(raw: string | null | undefined, fallback: T): T {
  if (!raw) return fallback;
  try { return JSON.parse(raw) as T; }
  catch { return fallback; }
}

type Row = Record<string, unknown>;

/** SQLite-backed data store with lazy initialization. */
class DataStore {
  private _db: Database.Database | null = null;

  private get db(): Database.Database {
    if (!this._db) {
      this._db = new Database(DB_PATH);
      initDatabase(this._db);
    }
    return this._db;
  }

  // ── Users ──

  get users(): AdminUser[] {
    return (this.db.prepare('SELECT * FROM users ORDER BY created_at DESC').all() as Row[]).map(r => ({
      id: r.id as string, email: r.email as string, name: r.name as string,
      role: r.role as AdminUser['role'], permissions: parseJson<Permission[]>(r.permissions as string, []),
      organizationId: r.organization_id as string, lastLoginAt: r.last_login_at as string | undefined,
      createdAt: r.created_at as string, isActive: Boolean(r.is_active),
      ssoProvider: (r.sso_provider as AdminUser['ssoProvider']) ?? undefined,
      mfaEnabled: Boolean(r.mfa_enabled),
    }));
  }

  set users(val: AdminUser[]) {
    const insert = this.db.prepare(`INSERT OR REPLACE INTO users (id, email, name, role, permissions, organization_id, last_login_at, created_at, is_active) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`);
    this.db.transaction(() => { for (const u of val) insert.run(u.id, u.email, u.name, u.role, JSON.stringify(u.permissions || []), u.organizationId || '', u.lastLoginAt || null, u.createdAt, u.isActive ? 1 : 0); })();
  }

  addUser(user: AdminUser & { passwordHash?: string }): void {
    this.db.prepare(`INSERT INTO users (id, email, name, role, password_hash, permissions, organization_id, created_at, is_active) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)`).run(
      user.id, user.email, user.name, user.role, user.passwordHash || null,
      JSON.stringify(user.permissions || []), user.organizationId || '', user.createdAt, user.isActive ? 1 : 0);
  }

  updateUser(id: string, updates: Partial<AdminUser>): boolean {
    const sets: string[] = []; const vals: unknown[] = [];
    if (updates.name !== undefined) { sets.push('name = ?'); vals.push(updates.name); }
    if (updates.role !== undefined) { sets.push('role = ?'); vals.push(updates.role); }
    if (updates.isActive !== undefined) { sets.push('is_active = ?'); vals.push(updates.isActive ? 1 : 0); }
    if (updates.permissions !== undefined) { sets.push('permissions = ?'); vals.push(JSON.stringify(updates.permissions)); }
    if (updates.lastLoginAt !== undefined) { sets.push('last_login_at = ?'); vals.push(updates.lastLoginAt); }
    if (sets.length === 0) return false;
    vals.push(id);
    return this.db.prepare(`UPDATE users SET ${sets.join(', ')} WHERE id = ?`).run(...vals).changes > 0;
  }

  deleteUser(id: string): boolean {
    return this.db.prepare('DELETE FROM users WHERE id = ?').run(id).changes > 0;
  }

  findUserByEmail(email: string): (AdminUser & { passwordHash?: string }) | undefined {
    const r = this.db.prepare('SELECT * FROM users WHERE email = ?').get(email) as Row | undefined;
    if (!r) return undefined;
    return {
      id: r.id as string, email: r.email as string, name: r.name as string,
      role: r.role as AdminUser['role'], passwordHash: r.password_hash as string | undefined,
      permissions: parseJson<Permission[]>(r.permissions as string, []),
      organizationId: r.organization_id as string,
      lastLoginAt: r.last_login_at as string | undefined, createdAt: r.created_at as string,
      isActive: Boolean(r.is_active),
    };
  }

  // ── Policies ──

  get policies(): PolicyRule[] {
    return (this.db.prepare('SELECT * FROM policies ORDER BY created_at DESC').all() as Row[]).map(r => ({
      id: r.id as string, domain: r.domain, name: r.name as string,
      description: (r.description as string) || '',
      assignmentTarget: r.assignment_target, assignmentValue: r.assignment_value as string,
      enforcementMode: r.enforcement_mode, configuration: parseJson(r.configuration as string, {}),
      createdBy: r.created_by as string, version: r.version as number, isActive: Boolean(r.is_active),
      createdAt: r.created_at as string, updatedAt: (r.updated_at as string) || r.created_at as string,
    }) as PolicyRule);
  }

  set policies(val: PolicyRule[]) {
    const insert = this.db.prepare(`INSERT OR REPLACE INTO policies (id, domain, name, assignment_target, assignment_value, enforcement_mode, configuration, created_by, version, is_active) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`);
    this.db.transaction(() => { for (const p of val) insert.run(p.id, p.domain, p.name, p.assignmentTarget || 'organization', p.assignmentValue || '', p.enforcementMode, JSON.stringify(p.configuration), p.createdBy || '', p.version || 1, p.isActive ? 1 : 0); })();
  }

  // ── Approvals ──

  get approvals(): ApprovalRequest[] {
    return (this.db.prepare('SELECT * FROM approvals ORDER BY requested_at DESC').all() as Row[]).map(r => ({
      id: r.id as string, operation: r.operation as string, trigger: r.trigger_type as string,
      source: r.source as string, destination: r.destination as string,
      fileCount: r.file_count as number, totalSize: r.total_size as number,
      state: r.state as string, requestedBy: r.requested_by as string,
      requestedAt: r.requested_at as string, reviewedBy: r.reviewed_by as string | undefined,
      reviewedAt: r.reviewed_at as string | undefined, reviewComment: r.review_comment as string | undefined,
      expiresAt: r.expires_at as string | undefined,
      reason: (r.reason as string) || '', triggeringPolicy: (r.triggering_policy as string) || '',
      requestedByName: (r.requested_by_name as string) || '', organizationId: (r.organization_id as string) || '',
    }) as ApprovalRequest);
  }

  set approvals(val: ApprovalRequest[]) {
    const insert = this.db.prepare(`INSERT OR REPLACE INTO approvals (id, operation, trigger_type, source, destination, file_count, total_size, state, requested_by, requested_at, reviewed_by, reviewed_at, review_comment, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`);
    this.db.transaction(() => { for (const a of val) insert.run(a.id, a.operation, a.trigger || '', a.source || '', a.destination || '', a.fileCount || 0, a.totalSize || 0, a.state, a.requestedBy || '', a.requestedAt, a.reviewedBy || null, a.reviewedAt || null, a.reviewComment || null, a.expiresAt || null); })();
  }

  // ── Audit Entries ──

  get auditEntries(): AuditEntry[] {
    return (this.db.prepare('SELECT * FROM audit_entries ORDER BY timestamp DESC').all() as Row[]).map(r => ({
      id: r.id as string, eventType: r.event_type as string, severity: r.severity as string,
      timestamp: r.timestamp as string, userId: r.user_id as string, userEmail: r.user_email as string,
      ipAddress: r.ip_address as string, userAgent: r.user_agent as string,
      description: r.description as string, metadata: parseJson(r.metadata as string, {}),
      integrityHash: r.integrity_hash as string, previousHash: r.previous_hash as string,
    }) as AuditEntry);
  }

  set auditEntries(val: AuditEntry[]) {
    const insert = this.db.prepare(`INSERT OR REPLACE INTO audit_entries (id, event_type, severity, timestamp, user_id, user_email, ip_address, user_agent, description, metadata, integrity_hash, previous_hash) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`);
    this.db.transaction(() => { for (const e of val) insert.run(e.id, e.eventType, e.severity, e.timestamp, e.userId || '', e.userEmail || '', e.ipAddress || '', e.userAgent || '', e.description, JSON.stringify(e.metadata || {}), e.integrityHash || '', e.previousHash || ''); })();
  }

  // ── Devices ──

  get devices(): DeviceHealth[] {
    return (this.db.prepare('SELECT * FROM devices ORDER BY last_seen DESC').all() as Row[]).map(r => ({
      id: r.id as string, deviceId: r.device_id as string, deviceName: r.device_name as string,
      userId: r.user_id as string, userEmail: r.user_email as string,
      userName: (r.user_name as string) || '', clientVersion: r.client_version as string,
      os: r.os as string, osVersion: (r.os_version as string) || '', arch: r.arch as string,
      status: r.status as string, lastSeen: r.last_seen as string,
      lastSeenAt: (r.last_seen as string) || '',
      ipAddress: r.ip_address as string, failurePatterns: parseJson(r.failure_patterns as string, []),
      policyCompliance: parseJson(r.policy_compliance as string, { isCompliant: true, violations: [], lastCheckedAt: '' }),
      cpuUsage: r.cpu_usage as number, memoryUsage: r.memory_usage as number, diskUsage: r.disk_usage as number,
    }) as unknown as DeviceHealth);
  }

  set devices(val: DeviceHealth[]) {
    const insert = this.db.prepare(`INSERT OR REPLACE INTO devices (id, device_id, device_name, user_id, user_name, client_version, os, os_version, status, last_seen, failure_patterns, policy_compliance, cpu_usage, memory_usage, disk_usage) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`);
    this.db.transaction(() => { for (const d of val) insert.run(d.id, d.deviceId, d.deviceName || '', d.userId || '', d.userName || '', d.clientVersion || '', d.os || '', d.osVersion || '', d.status, d.lastSeenAt || null, JSON.stringify(d.failurePatterns || []), JSON.stringify(d.policyCompliance || {}), d.cpuUsage || 0, d.memoryUsage || 0, d.diskUsage || 0); })();
  }

  // ── Workspaces ──

  get workspaces(): Workspace[] {
    return (this.db.prepare('SELECT * FROM workspaces ORDER BY created_at DESC').all() as Row[]).map(r => ({
      id: r.id as string, name: r.name as string, description: '',
      organizationId: r.organization_id as string,
      connections: parseJson(r.connections as string, []), syncTemplates: parseJson(r.sync_templates as string, []),
      automationTemplates: parseJson(r.automation_templates as string, []),
      members: parseJson(r.members as string, []),
      createdAt: r.created_at as string, updatedAt: r.created_at as string,
    }) as unknown as Workspace);
  }

  set workspaces(val: Workspace[]) {
    const insert = this.db.prepare(`INSERT OR REPLACE INTO workspaces (id, name, organization_id, connections, sync_templates, automation_templates, members) VALUES (?, ?, ?, ?, ?, ?, ?)`);
    this.db.transaction(() => { for (const w of val) insert.run(w.id, w.name, w.organizationId || '', JSON.stringify(w.connections || []), JSON.stringify(w.syncTemplates || []), JSON.stringify(w.automationTemplates || []), JSON.stringify(w.members || [])); })();
  }

  get sharedConnections(): SharedConnection[] {
    return (this.db.prepare('SELECT * FROM shared_connections ORDER BY created_at DESC').all() as Row[]).map(r => ({
      id: r.id as string, name: r.name as string, connectorType: r.connector_type as string,
      config: parseJson(r.config as string, {}), ownerId: r.owner_id as string,
      sharedWith: parseJson(r.shared_with as string, []), propagationStatus: r.propagation_status as string,
      ownerName: '', organizationId: '', createdAt: r.created_at as string, updatedAt: r.created_at as string, isActive: true,
    }) as unknown as SharedConnection);
  }

  set sharedConnections(val: SharedConnection[]) {
    const insert = this.db.prepare(`INSERT OR REPLACE INTO shared_connections (id, name, connector_type, config, owner_id, shared_with, propagation_status) VALUES (?, ?, ?, ?, ?, ?, ?)`);
    this.db.transaction(() => { for (const c of val) insert.run(c.id, c.name, c.connectorType || '', JSON.stringify(c.config || {}), c.ownerId || '', JSON.stringify(c.sharedWith || []), c.propagationStatus || 'active'); })();
  }

  get syncTemplates(): SyncTemplate[] {
    return (this.db.prepare('SELECT * FROM sync_templates ORDER BY created_at DESC').all() as Row[]).map(r => ({
      id: r.id as string, name: r.name as string, sourceConnector: r.source_connector as string,
      sourcePath: r.source_path as string, destConnector: r.dest_connector as string,
      destPath: r.dest_path as string, syncMode: r.sync_mode as string,
      conflictResolution: r.conflict_resolution as string, schedule: r.schedule as string,
      activatedByUsers: parseJson(r.activated_by_users as string, []),
      propagationStatus: r.propagation_status as string,
    }) as unknown as SyncTemplate);
  }

  set syncTemplates(val: SyncTemplate[]) {
    const insert = this.db.prepare(`INSERT OR REPLACE INTO sync_templates (id, name, source_connector, source_path, dest_connector, dest_path, sync_mode, conflict_resolution, schedule, activated_by_users, propagation_status) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`);
    this.db.transaction(() => { for (const t of val) insert.run(t.id, t.name, t.sourceConnector || '', t.sourcePath || '', t.destConnector || '', t.destPath || '', t.syncMode || 'one_way', t.conflictResolution || 'newest_wins', t.schedule || '', JSON.stringify(t.activatedByUsers || []), t.propagationStatus || 'active'); })();
  }
}

/** Singleton data store instance. Backed by SQLite — data persists across restarts. */
export const dataStore = new DataStore();
