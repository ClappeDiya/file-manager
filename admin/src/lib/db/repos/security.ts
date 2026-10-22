/**
 * Per-organization security settings (password age, and the settings the
 * sign-in path enforces) and single-use password tokens (self-service reset
 * and forced change after expiry).
 */
import { db, hashToken, newToken, now, parseJson, type Row } from '../index';
import { membershipsOf, OrgError } from './orgs';
import type { UserRecord } from './users';
import { hasPasskeys } from './passkeys';

export interface SecuritySettings {
  /** Days before a console password must be changed (0 = never). */
  passwordMaxAgeDays: number;
  /** Every member must use two-factor sign-in (TOTP). */
  requireMfa: boolean;
  /** With requireMfa: single sign-on sessions are exempt (the identity provider enforces MFA). */
  ssoExemptFromMfa: boolean;
  /** Sign out sessions after this many minutes without activity (0 = only the 8-hour limit). */
  sessionIdleMinutes: number;
}

export const DEFAULT_SECURITY: SecuritySettings = { passwordMaxAgeDays: 0, requireMfa: false, ssoExemptFromMfa: false, sessionIdleMinutes: 0 };

export function getSecurity(orgId: string): SecuritySettings {
  const r = db().prepare('SELECT settings FROM org_security WHERE org_id = ?').get(orgId) as Row | undefined;
  return { ...DEFAULT_SECURITY, ...parseJson<Partial<SecuritySettings>>(r?.settings, {}) };
}

export function setSecurity(orgId: string, patch: Partial<SecuritySettings>, by: string): SecuritySettings {
  const next = { ...getSecurity(orgId), ...patch };
  if (!Number.isInteger(next.passwordMaxAgeDays) || next.passwordMaxAgeDays < 0 || next.passwordMaxAgeDays > 3650) throw new OrgError('Password age is 0 (never) to 3650 days');
  if (typeof next.requireMfa !== 'boolean' || typeof next.ssoExemptFromMfa !== 'boolean') throw new OrgError('requireMfa and ssoExemptFromMfa are true or false');
  if (!Number.isInteger(next.sessionIdleMinutes) || (next.sessionIdleMinutes !== 0 && (next.sessionIdleMinutes < 5 || next.sessionIdleMinutes > 480))) throw new OrgError('The idle timeout is 0 (off) or 5 to 480 minutes');
  db()
    .prepare('INSERT INTO org_security (org_id, settings, updated_by, updated_at) VALUES (?, ?, ?, ?) ON CONFLICT(org_id) DO UPDATE SET settings = excluded.settings, updated_by = excluded.updated_by, updated_at = excluded.updated_at')
    .run(orgId, JSON.stringify(next), by, now());
  return next;
}

/** The strictest idle timeout over the account's organizations, in minutes (0 = none). */
export function sessionIdleMinutes(userId: string): number {
  const v = membershipsOf(userId)
    .map((m) => getSecurity(m.orgId).sessionIdleMinutes)
    .filter((d) => d > 0);
  return v.length ? Math.min(...v) : 0;
}

/** The strictest password age over the account's organizations (0 = no expiry). */
export function passwordMaxAgeDays(user: UserRecord): number {
  const ages = membershipsOf(user.id)
    .map((m) => getSecurity(m.orgId).passwordMaxAgeDays)
    .filter((d) => d > 0);
  return ages.length ? Math.min(...ages) : 0;
}

/**
 * Whether the account must use two-factor sign-in: any of its organizations
 * requires it, and (for an SSO session) not every requiring organization
 * exempts single sign-on.
 */
export function mfaRequired(user: UserRecord, viaSso: boolean): boolean {
  const requiring = membershipsOf(user.id)
    .map((m) => getSecurity(m.orgId))
    .filter((s) => s.requireMfa);
  if (!requiring.length) return false;
  return !(viaSso && requiring.every((s) => s.ssoExemptFromMfa));
}

/** Required two-factor sign-in that the account has not set up (no TOTP, no passkey). */
export function needsSecondFactorSetup(user: UserRecord, viaSso: boolean): boolean {
  return !user.mfaEnabled && !hasPasskeys(user.id) && mfaRequired(user, viaSso);
}

export function passwordExpired(user: UserRecord, at = Date.now()): boolean {
  const days = passwordMaxAgeDays(user);
  if (!days || !user.passwordHash) return false;
  const since = new Date(user.passwordChangedAt ?? user.createdAt).getTime();
  return at - since > days * 86_400_000;
}

export type PasswordTokenPurpose = 'reset' | 'expired';
const TTL_MINUTES: Record<PasswordTokenPurpose, number> = { reset: 60, expired: 15 };

/** A single-use token; older unused ones for the user are invalidated. */
export function createPasswordToken(userId: string, purpose: PasswordTokenPurpose): string {
  const token = newToken('ufop_pwd', 24);
  db().prepare('UPDATE password_resets SET used_at = ? WHERE user_id = ? AND used_at IS NULL').run(now(), userId);
  db()
    .prepare('INSERT INTO password_resets (token_hash, user_id, purpose, created_at, expires_at) VALUES (?, ?, ?, ?, ?)')
    .run(hashToken(token), userId, purpose, now(), new Date(Date.now() + TTL_MINUTES[purpose] * 60_000).toISOString());
  return token;
}

/** Use a token (once); returns the user id, or null when unknown, used or expired. */
export function consumePasswordToken(token: string): { userId: string; purpose: PasswordTokenPurpose } | null {
  const r = db().prepare('SELECT * FROM password_resets WHERE token_hash = ?').get(hashToken(token)) as Row | undefined;
  if (!r || r.used_at || new Date(r.expires_at as string) <= new Date()) return null;
  const ok = db().prepare('UPDATE password_resets SET used_at = ? WHERE token_hash = ? AND used_at IS NULL').run(now(), r.token_hash).changes === 1;
  return ok ? { userId: r.user_id as string, purpose: r.purpose as PasswordTokenPurpose } : null;
}

/** Whether a token is valid (without using it), for the reset page. */
export function peekPasswordToken(token: string): { userId: string; purpose: PasswordTokenPurpose } | null {
  const r = db().prepare('SELECT * FROM password_resets WHERE token_hash = ?').get(hashToken(token)) as Row | undefined;
  if (!r || r.used_at || new Date(r.expires_at as string) <= new Date()) return null;
  return { userId: r.user_id as string, purpose: r.purpose as PasswordTokenPurpose };
}
