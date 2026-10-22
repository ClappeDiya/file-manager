/** Console users (accounts). Roles per organization live in memberships (see orgs.ts). */
import { db, newId, now, parseJson, type Row } from '../index';
import type { AdminUser, Permission, Role } from '../../types/auth';

export interface UserRecord extends AdminUser {
  passwordHash: string | null;
  mfaEnabled: boolean;
  totpSecretCiphertext: string | null;
  totpPendingCiphertext: string | null;
  recoveryCodes: string[];
  lastTotpStep: number | null;
}

function fromRow(r: Row): UserRecord {
  return {
    id: r.id as string,
    email: r.email as string,
    name: r.name as string,
    role: r.role as Role,
    permissions: parseJson<Permission[]>(r.permissions, []),
    organizationId: (r.organization_id as string) || 'org_default',
    lastLoginAt: (r.last_login_at as string) ?? undefined,
    createdAt: r.created_at as string,
    isActive: Boolean(r.is_active),
    ssoProvider: (r.sso_provider as AdminUser['ssoProvider']) ?? null,
    passwordHash: (r.password_hash as string) ?? null,
    mfaEnabled: Boolean(r.mfa_enabled),
    totpSecretCiphertext: (r.totp_secret_ciphertext as string) ?? null,
    totpPendingCiphertext: (r.totp_pending_ciphertext as string) ?? null,
    recoveryCodes: parseJson<string[]>(r.recovery_codes, []),
    lastTotpStep: (r.last_totp_step as number) ?? null,
  };
}

/** The public shape (no secrets). */
export function publicUser(u: UserRecord): AdminUser & { mfaEnabled: boolean } {
  return { id: u.id, email: u.email, name: u.name, role: u.role, permissions: u.permissions, organizationId: u.organizationId, lastLoginAt: u.lastLoginAt, createdAt: u.createdAt, isActive: u.isActive, ssoProvider: u.ssoProvider, mfaEnabled: u.mfaEnabled };
}

export function getUser(id: string): UserRecord | null {
  const r = db().prepare('SELECT * FROM users WHERE id = ?').get(id) as Row | undefined;
  return r ? fromRow(r) : null;
}

export function findUserByEmail(email: string): UserRecord | null {
  const r = db().prepare('SELECT * FROM users WHERE lower(email) = lower(?)').get(email.trim()) as Row | undefined;
  return r ? fromRow(r) : null;
}

export function listUsers(ids?: string[]): UserRecord[] {
  if (ids && ids.length === 0) return [];
  const rows = ids
    ? (db().prepare(`SELECT * FROM users WHERE id IN (${ids.map(() => '?').join(',')}) ORDER BY lower(name)`).all(...ids) as Row[])
    : (db().prepare('SELECT * FROM users ORDER BY lower(name)').all() as Row[]);
  return rows.map(fromRow);
}

export interface NewUser {
  email: string;
  name?: string;
  role: Role;
  organizationId: string;
  passwordHash?: string | null;
  ssoProvider?: AdminUser['ssoProvider'];
}

export function createUser(u: NewUser): UserRecord {
  const id = newId('usr');
  const email = u.email.trim().toLowerCase();
  db()
    .prepare(`INSERT INTO users (id, email, name, role, password_hash, permissions, organization_id, created_at, is_active, sso_provider) VALUES (?, ?, ?, ?, ?, '[]', ?, ?, 1, ?)`)
    .run(id, email, u.name?.trim() || email.split('@')[0], u.role, u.passwordHash ?? null, u.organizationId, now(), u.ssoProvider ?? null);
  return getUser(id)!;
}

export interface UserUpdate {
  email?: string;
  name?: string;
  role?: Role;
  isActive?: boolean;
  passwordHash?: string | null;
  lastLoginAt?: string;
  organizationId?: string;
  mfaEnabled?: boolean;
  totpSecretCiphertext?: string | null;
  totpPendingCiphertext?: string | null;
  recoveryCodes?: string[];
  lastTotpStep?: number | null;
}

const COLUMNS: Record<keyof UserUpdate, string> = {
  email: 'email',
  name: 'name',
  role: 'role',
  isActive: 'is_active',
  passwordHash: 'password_hash',
  lastLoginAt: 'last_login_at',
  organizationId: 'organization_id',
  mfaEnabled: 'mfa_enabled',
  totpSecretCiphertext: 'totp_secret_ciphertext',
  totpPendingCiphertext: 'totp_pending_ciphertext',
  recoveryCodes: 'recovery_codes',
  lastTotpStep: 'last_totp_step',
};

export function updateUser(id: string, u: UserUpdate): boolean {
  const sets: string[] = [];
  const vals: unknown[] = [];
  for (const [k, col] of Object.entries(COLUMNS) as [keyof UserUpdate, string][]) {
    const v = u[k];
    if (v === undefined) continue;
    sets.push(`${col} = ?`);
    vals.push(typeof v === 'boolean' ? (v ? 1 : 0) : Array.isArray(v) ? JSON.stringify(v) : v);
  }
  if (sets.length === 0) return false;
  return db().prepare(`UPDATE users SET ${sets.join(', ')} WHERE id = ?`).run(...vals, id).changes > 0;
}

export function deleteUser(id: string): boolean {
  return db().prepare('DELETE FROM users WHERE id = ?').run(id).changes > 0;
}
