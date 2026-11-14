/**
 * SCIM 2.0 provisioning storage: per-organization bearer tokens (hashed,
 * shown once) and links from console users/groups to the identity
 * provider's `externalId`, with SCIM timestamps.
 */
import { db, hashToken, newId, newToken, now, type Row } from '../index';
import type { Role } from '../../types/auth';
import { OrgError } from './orgs';

export type ScimResource = 'User' | 'Group';
export const SCIM_ROLES: Role[] = ['user', 'read_only_auditor', 'operator', 'approver'];

export interface ScimToken {
  id: string;
  orgId: string;
  name: string;
  tokenPrefix: string;
  defaultRole: Role;
  createdBy: string;
  createdAt: string;
  lastUsedAt: string | null;
  revokedAt: string | null;
}

function tokenFromRow(r: Row): ScimToken {
  return {
    id: r.id as string,
    orgId: r.org_id as string,
    name: r.name as string,
    tokenPrefix: r.token_prefix as string,
    defaultRole: r.default_role as Role,
    createdBy: r.created_by as string,
    createdAt: r.created_at as string,
    lastUsedAt: (r.last_used_at as string) ?? null,
    revokedAt: (r.revoked_at as string) ?? null,
  };
}

/** Create a token; the secret is returned once. Provisioned users get `defaultRole` (never an admin role). */
export function createScimToken(orgId: string, name: string, defaultRole: Role, by: string): { token: ScimToken; secret: string } {
  if (!name.trim()) throw new OrgError('Give the token a name');
  if (!SCIM_ROLES.includes(defaultRole)) throw new OrgError(`Provisioned users can get the role ${SCIM_ROLES.join(', ')}`);
  const secret = newToken('ufop_scim', 32);
  const id = newId('sct');
  db()
    .prepare('INSERT INTO scim_tokens (id, org_id, name, token_hash, token_prefix, default_role, created_by, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)')
    .run(id, orgId, name.trim().slice(0, 100), hashToken(secret), secret.slice(0, 14), defaultRole, by, now());
  return { token: tokenFromRow(db().prepare('SELECT * FROM scim_tokens WHERE id = ?').get(id) as Row), secret };
}

export function listScimTokens(orgId: string): ScimToken[] {
  return (db().prepare('SELECT * FROM scim_tokens WHERE org_id = ? ORDER BY created_at DESC').all(orgId) as Row[]).map(tokenFromRow);
}

export function revokeScimToken(orgId: string, id: string): boolean {
  return db().prepare('UPDATE scim_tokens SET revoked_at = ? WHERE org_id = ? AND id = ? AND revoked_at IS NULL').run(now(), orgId, id).changes > 0;
}

/** The active token for a bearer secret (and mark it used). */
export function scimTokenFor(secret: string): ScimToken | null {
  if (!secret.startsWith('ufop_scim_')) return null;
  const r = db().prepare('SELECT * FROM scim_tokens WHERE token_hash = ? AND revoked_at IS NULL').get(hashToken(secret)) as Row | undefined;
  if (!r) return null;
  db().prepare('UPDATE scim_tokens SET last_used_at = ? WHERE id = ?').run(now(), r.id);
  return tokenFromRow(r);
}

export interface ScimLink {
  resourceId: string;
  externalId: string | null;
  createdAt: string;
  updatedAt: string;
}

export function getLink(orgId: string, type: ScimResource, resourceId: string): ScimLink | null {
  const r = db().prepare('SELECT * FROM scim_links WHERE org_id = ? AND resource_type = ? AND resource_id = ?').get(orgId, type, resourceId) as Row | undefined;
  return r ? { resourceId: r.resource_id as string, externalId: (r.external_id as string) ?? null, createdAt: r.created_at as string, updatedAt: r.updated_at as string } : null;
}

export function findByExternalId(orgId: string, type: ScimResource, externalId: string): string | null {
  const r = db().prepare('SELECT resource_id FROM scim_links WHERE org_id = ? AND resource_type = ? AND external_id = ?').get(orgId, type, externalId) as { resource_id: string } | undefined;
  return r?.resource_id ?? null;
}

/** Record (or touch) a link; `externalId` undefined keeps the stored one. */
export function touchLink(orgId: string, type: ScimResource, resourceId: string, externalId?: string | null): void {
  const ts = now();
  const cur = getLink(orgId, type, resourceId);
  if (externalId !== undefined && externalId !== null && externalId !== cur?.externalId) {
    const other = findByExternalId(orgId, type, externalId);
    if (other && other !== resourceId) throw new OrgError(`externalId '${externalId}' is used by another ${type.toLowerCase()}`, 409);
  }
  if (cur) {
    db().prepare('UPDATE scim_links SET external_id = ?, updated_at = ? WHERE org_id = ? AND resource_type = ? AND resource_id = ?').run(externalId === undefined ? cur.externalId : externalId, ts, orgId, type, resourceId);
  } else {
    db().prepare('INSERT INTO scim_links (org_id, resource_type, resource_id, external_id, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)').run(orgId, type, resourceId, externalId ?? null, ts, ts);
  }
}

export function dropLink(orgId: string, type: ScimResource, resourceId: string): void {
  db().prepare('DELETE FROM scim_links WHERE org_id = ? AND resource_type = ? AND resource_id = ?').run(orgId, type, resourceId);
}
