/**
 * In-memory data store for the admin console.
 *
 * In production, this would be backed by a real database (PostgreSQL via DATABASE_URL).
 * For development/demo without a database, data starts empty and can be populated
 * via API calls. Data resets on server restart.
 *
 * When DATABASE_URL is configured, all store methods should be replaced with
 * actual database queries.
 */

import type { AdminUser } from './types/auth';
import type { PolicyRule } from './types/policies';
import type { ApprovalRequest } from './types/approvals';
import type { AuditEntry, DeviceHealth } from './types/audit';
import type { SharedConnection, SyncTemplate, Workspace } from './types/workspaces';

/** Runtime data store - populated via API or seeded from database */
class DataStore {
  users: AdminUser[] = [];
  policies: PolicyRule[] = [];
  approvals: ApprovalRequest[] = [];
  auditEntries: AuditEntry[] = [];
  devices: DeviceHealth[] = [];
  workspaces: Workspace[] = [];
  sharedConnections: SharedConnection[] = [];
  syncTemplates: SyncTemplate[] = [];

  constructor() {
    // In production with DATABASE_URL, this would load from DB.
    // Without it, start with empty collections.
    // Data is populated via the admin API endpoints.
  }
}

/**
 * Singleton data store instance.
 * Persists across requests within the same server process.
 */
export const dataStore = new DataStore();
