/**
 * Personal API tokens for the console API: `Authorization: Bearer ufop_cat_…`.
 *
 * A token belongs to one user in one organization and carries a subset of
 * the permissions its owner had when it was created. On every request the
 * effective permissions are that subset intersected with the owner's role
 * *now*, so demoting or removing the owner narrows or ends the token. Every
 * token expires (1–365 days); only a hash is stored.
 */
import { db, hashToken, newId, newToken, now, parseJson, type Row } from '../index';
import { OrgError } from './orgs';
import type { Permission } from '../../types/auth';

export const TOKEN_PREFIX = 'ufop_cat';
export const MAX_DAYS = 365;
const MAX_PER_USER = 50;

export interface ApiToken {
  id: string;
  orgId: string;
  userId: string;
  name: string;
  /** The first characters, to recognise a token. */
  prefix: string;
  permissions: Permission[];
  createdAt: string;
  expiresAt: string;
  lastUsedAt: string | null;
  lastUsedIp: string | null;
  revokedAt: string | null;
}

function fromRow(r: Row): ApiToken {
  return {
    id: r.id as string,
    orgId: r.org_id as string,
    userId: r.user_id as string,
    name: r.name as string,
    prefix: r.prefix as string,
    permissions: parseJson<Permission[]>(r.permissions, []),
    createdAt: r.created_at as string,
    expiresAt: r.expires_at as string,
    lastUsedAt: (r.last_used_at as string) ?? null,
    lastUsedIp: (r.last_used_ip as string) ?? null,
    revokedAt: (r.revoked_at as string) ?? null,
  };
}

export const isBearerToken = (v: string | null | undefined) => !!v && v.startsWith(`Bearer ${TOKEN_PREFIX}_`);

/** Create a token; the secret is returned once. `allowed` = the creator's current permissions. */
export function createApiToken(o: { orgId: string; userId: string; name: string; permissions: unknown; days: unknown; allowed: Permission[] }): { token: ApiToken; secret: string } {
  const name = typeof o.name === 'string' ? o.name.trim() : '';
  if (!name || name.length > 80) throw new OrgError('A token needs a name of 1–80 characters');
  const days = Number(o.days);
  if (!Number.isInteger(days) || days < 1 || days > MAX_DAYS) throw new OrgError(`Tokens expire after 1 to ${MAX_DAYS} days`);
  if (!Array.isArray(o.permissions) || !o.permissions.length || o.permissions.some((p) => typeof p !== 'string')) throw new OrgError('Choose at least one permission');
  const perms = [...new Set(o.permissions as Permission[])];
  const extra = perms.filter((p) => !o.allowed.includes(p));
  if (extra.length) throw new OrgError(`You cannot grant permissions you do not have: ${extra.join(', ')}`);
  const active = db().prepare('SELECT COUNT(*) AS n FROM api_tokens WHERE user_id = ? AND revoked_at IS NULL AND expires_at > ?').get(o.userId, now()) as { n: number };
  if (active.n >= MAX_PER_USER) throw new OrgError(`At most ${MAX_PER_USER} active tokens; revoke unused ones first`);
  const secret = newToken(TOKEN_PREFIX, 32);
  const id = newId('tok');
  db()
    .prepare('INSERT INTO api_tokens (id, org_id, user_id, name, token_hash, prefix, permissions, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)')
    .run(id, o.orgId, o.userId, name, hashToken(secret), secret.slice(0, 16), JSON.stringify(perms), now(), new Date(Date.now() + days * 86_400_000).toISOString());
  return { token: getApiToken(id)!, secret };
}

export function getApiToken(id: string): ApiToken | null {
  const r = db().prepare('SELECT * FROM api_tokens WHERE id = ?').get(id) as Row | undefined;
  return r ? fromRow(r) : null;
}

/** A user's tokens in an organization (or every member's, with `userId` null), newest first. */
export function listApiTokens(orgId: string, userId: string | null): ApiToken[] {
  const rows = userId
    ? db().prepare('SELECT * FROM api_tokens WHERE org_id = ? AND user_id = ? ORDER BY created_at DESC').all(orgId, userId)
    : db().prepare('SELECT * FROM api_tokens WHERE org_id = ? ORDER BY created_at DESC').all(orgId);
  return (rows as Row[]).map(fromRow);
}

export function revokeApiToken(id: string): boolean {
  return db().prepare('UPDATE api_tokens SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL').run(now(), id).changes > 0;
}

/** Revoke every token of a user (deactivation, password reset by an admin). */
export function revokeUserApiTokens(userId: string): number {
  return db().prepare('UPDATE api_tokens SET revoked_at = ? WHERE user_id = ? AND revoked_at IS NULL').run(now(), userId).changes;
}

/** The valid token behind a secret (not revoked, not expired); records its use. */
export function useApiToken(secret: string, ip: string): ApiToken | null {
  const r = db().prepare('SELECT * FROM api_tokens WHERE token_hash = ?').get(hashToken(secret)) as Row | undefined;
  if (!r) return null;
  const t = fromRow(r);
  if (t.revokedAt || new Date(t.expiresAt) <= new Date()) return null;
  db().prepare('UPDATE api_tokens SET last_used_at = ?, last_used_ip = ? WHERE id = ?').run(now(), ip.slice(0, 100), t.id);
  return t;
}
