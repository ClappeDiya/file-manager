/** Shared workspaces, connections and sync templates (pre-existing admin feature). */
import { db, parseJson, type Row } from '../index';
import type { SharedConnection, SyncTemplate, Workspace } from '../../types/workspaces';

export function listWorkspaces(): Workspace[] {
  return (db().prepare('SELECT * FROM workspaces ORDER BY created_at DESC').all() as Row[]).map(
    (r) =>
      ({
        id: r.id as string,
        name: r.name as string,
        description: '',
        organizationId: r.organization_id as string,
        connections: parseJson(r.connections, []),
        syncTemplates: parseJson(r.sync_templates, []),
        automationTemplates: parseJson(r.automation_templates, []),
        members: parseJson(r.members, []),
        createdAt: r.created_at as string,
        updatedAt: r.created_at as string,
      }) as unknown as Workspace,
  );
}

export function listSharedConnections(): SharedConnection[] {
  return (db().prepare('SELECT * FROM shared_connections ORDER BY created_at DESC').all() as Row[]).map(
    (r) =>
      ({
        id: r.id as string,
        name: r.name as string,
        connectorType: r.connector_type as string,
        config: parseJson(r.config, {}),
        ownerId: r.owner_id as string,
        sharedWith: parseJson(r.shared_with, []),
        propagationStatus: r.propagation_status as string,
        ownerName: '',
        organizationId: '',
        createdAt: r.created_at as string,
        updatedAt: r.created_at as string,
        isActive: true,
      }) as unknown as SharedConnection,
  );
}

export function listSyncTemplates(): SyncTemplate[] {
  return (db().prepare('SELECT * FROM sync_templates ORDER BY created_at DESC').all() as Row[]).map(
    (r) =>
      ({
        id: r.id as string,
        name: r.name as string,
        sourceConnector: r.source_connector as string,
        sourcePath: r.source_path as string,
        destConnector: r.dest_connector as string,
        destPath: r.dest_path as string,
        syncMode: r.sync_mode as string,
        conflictResolution: r.conflict_resolution as string,
        schedule: r.schedule as string,
        activatedByUsers: parseJson(r.activated_by_users, []),
        propagationStatus: r.propagation_status as string,
      }) as unknown as SyncTemplate,
  );
}
