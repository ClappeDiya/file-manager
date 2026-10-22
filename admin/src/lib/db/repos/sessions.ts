/**
 * Console sessions. The session cookie (a JWT) names a row here by `sid`;
 * every request checks that the row is still valid, so sessions can be
 * listed and revoked, and an organization can sign out idle sessions.
 */
import { db, newId, now, type Row } from '../index';

export interface Session {
  id: string;
  userId: string;
  method: string;
  ip: string;
  userAgent: string;
  createdAt: string;
  lastSeenAt: string;
  /** When the user last proved who they are (sign-in or re-authentication). */
  verifiedAt: string;
  expiresAt: string;
  revokedAt: string | null;
  revokedReason: string | null;
  /** Viewing the console read-only as this user, until `viewAsUntil`. */
  viewAsUserId: string | null;
  viewAsUntil: string | null;
}

function fromRow(r: Row): Session {
  return {
    id: r.id as string,
    userId: r.user_id as string,
    method: r.method as string,
    ip: r.ip as string,
    userAgent: r.user_agent as string,
    createdAt: r.created_at as string,
    lastSeenAt: r.last_seen_at as string,
    verifiedAt: ((r.verified_at as string) ?? r.created_at) as string,
    expiresAt: r.expires_at as string,
    revokedAt: (r.revoked_at as string) ?? null,
    revokedReason: (r.revoked_reason as string) ?? null,
    viewAsUserId: (r.view_as_user_id as string) ?? null,
    viewAsUntil: (r.view_as_until as string) ?? null,
  };
}

/** last_seen_at is written at most this often per session. */
const TOUCH_MS = 60_000;

export function createSession(userId: string, o: { method?: string; ip?: string; userAgent?: string; hours: number }): Session {
  const id = newId('ses');
  const ts = now();
  db()
    .prepare('INSERT INTO sessions (id, user_id, method, ip, user_agent, created_at, last_seen_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)')
    .run(id, userId, (o.method ?? '').slice(0, 60), (o.ip ?? '').slice(0, 100), (o.userAgent ?? '').slice(0, 300), ts, ts, new Date(Date.now() + o.hours * 3600_000).toISOString());
  // Housekeeping: forget long-gone sessions.
  db().prepare('DELETE FROM sessions WHERE expires_at < ?').run(new Date(Date.now() - 30 * 86_400_000).toISOString());
  return getSession(id)!;
}

export function getSession(id: string): Session | null {
  const r = db().prepare('SELECT * FROM sessions WHERE id = ?').get(id) as Row | undefined;
  return r ? fromRow(r) : null;
}

/** Record activity (throttled). */
export function touchSession(s: Session, at = Date.now()): void {
  if (at - new Date(s.lastSeenAt).getTime() >= TOUCH_MS) db().prepare('UPDATE sessions SET last_seen_at = ? WHERE id = ?').run(new Date(at).toISOString(), s.id);
}

/** The user re-authenticated in this session (step-up). */
/** Start (or replace) viewing as `userId` until `until`. */
export function startViewAs(id: string, userId: string, until: Date): void {
  db().prepare('UPDATE sessions SET view_as_user_id = ?, view_as_until = ? WHERE id = ?').run(userId, until.toISOString(), id);
}

export function stopViewAs(id: string): void {
  db().prepare('UPDATE sessions SET view_as_user_id = NULL, view_as_until = NULL WHERE id = ?').run(id);
}

export function markVerified(id: string, at = new Date()): void {
  db().prepare('UPDATE sessions SET verified_at = ? WHERE id = ?').run(at.toISOString(), id);
}

export function revokeSession(id: string, reason: string): boolean {
  return db().prepare('UPDATE sessions SET revoked_at = ?, revoked_reason = ? WHERE id = ? AND revoked_at IS NULL').run(now(), reason.slice(0, 100), id).changes > 0;
}

/** Revoke every active session of a user (optionally keeping one); returns how many. */
export function revokeUserSessions(userId: string, reason: string, exceptId?: string): number {
  return db()
    .prepare('UPDATE sessions SET revoked_at = ?, revoked_reason = ? WHERE user_id = ? AND revoked_at IS NULL AND id != ?')
    .run(now(), reason.slice(0, 100), userId, exceptId ?? '').changes;
}

/** Active (not revoked, not expired) sessions, newest activity first. */
export function activeSessions(userId: string): Session[] {
  return (db().prepare('SELECT * FROM sessions WHERE user_id = ? AND revoked_at IS NULL AND expires_at > ? ORDER BY last_seen_at DESC').all(userId, now()) as Row[]).map(fromRow);
}
