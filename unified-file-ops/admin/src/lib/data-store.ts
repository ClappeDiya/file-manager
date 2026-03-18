/**
 * Database-backed data store for the admin console.
 *
 * This module provides backward-compatible access to all data entities
 * via the same `dataStore` export name. Under the hood, all reads and
 * writes go through SQLite (see db.ts).
 *
 * API routes can import either:
 *   - `dataStore` from here (backward-compatible getters)
 *   - Individual functions from `./db` (preferred for new code)
 */

import * as db from './db';

/**
 * Backward-compatible data store.
 * Properties return live database reads via getters.
 */
export const dataStore = {
  get users() { return db.getUsers(); },
  get policies() { return db.getPolicies(); },
  get approvals() { return db.getApprovals(); },
  get auditEntries() { return db.getAuditEntries(); },
  get devices() { return db.getDevices(); },
  get workspaces() { return db.getWorkspaces(); },
  get sharedConnections() { return db.getSharedConnections(); },
  get syncTemplates() { return db.getSyncTemplates(); },
};
