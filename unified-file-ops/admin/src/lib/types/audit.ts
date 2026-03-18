/**
 * Audit logging types (T-052).
 * Immutable audit entries covering all tracked event types.
 */

/** All audit event types */
export type AuditEventType =
  | 'auth.login'
  | 'auth.logout'
  | 'auth.login_failed'
  | 'connection.create'
  | 'connection.update'
  | 'connection.delete'
  | 'connection.test'
  | 'transfer.start'
  | 'transfer.complete'
  | 'transfer.fail'
  | 'transfer.cancel'
  | 'sync.start'
  | 'sync.complete'
  | 'sync.fail'
  | 'sync.conflict'
  | 'naming.translation'
  | 'naming.override'
  | 'approval.request'
  | 'approval.approve'
  | 'approval.deny'
  | 'approval.expire'
  | 'policy.create'
  | 'policy.update'
  | 'policy.delete'
  | 'policy.violation'
  | 'admin.user_invite'
  | 'admin.user_deactivate'
  | 'admin.user_remove'
  | 'admin.role_change'
  | 'admin.sso_configure'
  | 'admin.settings_change';

export const AUDIT_EVENT_CATEGORIES: Record<string, AuditEventType[]> = {
  Authentication: ['auth.login', 'auth.logout', 'auth.login_failed'],
  Connections: ['connection.create', 'connection.update', 'connection.delete', 'connection.test'],
  Transfers: ['transfer.start', 'transfer.complete', 'transfer.fail', 'transfer.cancel'],
  Sync: ['sync.start', 'sync.complete', 'sync.fail', 'sync.conflict'],
  Naming: ['naming.translation', 'naming.override'],
  Approvals: ['approval.request', 'approval.approve', 'approval.deny', 'approval.expire'],
  Policies: ['policy.create', 'policy.update', 'policy.delete', 'policy.violation'],
  Admin: ['admin.user_invite', 'admin.user_deactivate', 'admin.user_remove', 'admin.role_change', 'admin.sso_configure', 'admin.settings_change'],
};

export type AuditSeverity = 'info' | 'warning' | 'error' | 'critical';

/** Immutable audit log entry */
export interface AuditEntry {
  id: string;
  /** Event type from the taxonomy */
  eventType: AuditEventType;
  /** Severity level */
  severity: AuditSeverity;
  /** ISO timestamp */
  timestamp: string;
  /** User who performed the action */
  userId: string;
  userName: string;
  userEmail: string;
  /** Organization */
  organizationId: string;
  /** IP address of the action origin */
  ipAddress: string;
  /** User agent string */
  userAgent: string;
  /** Human-readable description */
  description: string;
  /** Structured metadata */
  metadata: Record<string, unknown>;
  /** Resource affected */
  resourceType?: string;
  resourceId?: string;
  /** Integrity hash (SHA-256 of entry content) */
  integrityHash: string;
  /** Previous entry hash for chain integrity */
  previousHash?: string;
}

/** Search/filter criteria for audit explorer */
export interface AuditFilter {
  eventTypes?: AuditEventType[];
  severities?: AuditSeverity[];
  userId?: string;
  dateFrom?: string;
  dateTo?: string;
  searchText?: string;
  resourceType?: string;
  resourceId?: string;
  page?: number;
  pageSize?: number;
}

/** Export formats */
export type AuditExportFormat = 'csv' | 'json' | 'syslog';

/** SIEM integration config (Enterprise) */
export interface SIEMConfig {
  type: 'webhook' | 'syslog';
  endpoint: string;
  format: 'json' | 'cef' | 'leef';
  enabled: boolean;
  authToken?: string;
  tlsEnabled: boolean;
  batchSize: number;
  flushIntervalSeconds: number;
}

/** Device health types */
export interface DeviceHealth {
  id: string;
  deviceId: string;
  deviceName: string;
  userId: string;
  userName: string;
  clientVersion: string;
  os: string;
  osVersion: string;
  lastSeenAt: string;
  status: 'online' | 'offline' | 'degraded' | 'error';
  /** Recent failure patterns */
  failurePatterns: FailurePattern[];
  /** Policy compliance status */
  policyCompliance: ComplianceStatus;
  /** Resource usage */
  cpuUsage?: number;
  memoryUsage?: number;
  diskUsage?: number;
}

export interface FailurePattern {
  type: string;
  count: number;
  lastOccurrence: string;
  description: string;
}

export interface ComplianceStatus {
  isCompliant: boolean;
  violations: PolicyViolation[];
  lastCheckedAt: string;
}

export interface PolicyViolation {
  policyId: string;
  policyName: string;
  domain: string;
  description: string;
  severity: 'low' | 'medium' | 'high' | 'critical';
  detectedAt: string;
}
