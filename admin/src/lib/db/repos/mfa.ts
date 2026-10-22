/** Pending second-factor challenges between password/SSO sign-in and the session. */
import { db, newId, now, type Row } from '../index';
import { open, seal } from '../../crypto';
import { hashRecoveryCode, newRecoveryCodes, newTotpSecret, verifyTotp } from '../../totp';
import { getUser, updateUser, type UserRecord } from './users';

const CHALLENGE_MINUTES = 5;
const MAX_ATTEMPTS = 5;

export interface Challenge {
  id: string;
  userId: string;
  orgId: string | null;
  method: string;
  attempts: number;
  expiresAt: string;
}

export function createChallenge(userId: string, orgId: string | null, method: string): Challenge {
  const id = newId('mfa');
  const expiresAt = new Date(Date.now() + CHALLENGE_MINUTES * 60_000).toISOString();
  db().prepare('INSERT INTO mfa_challenges (id, user_id, org_id, method, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?)').run(id, userId, orgId, method, now(), expiresAt);
  return { id, userId, orgId, method, attempts: 0, expiresAt };
}

function getChallenge(id: string): Challenge | null {
  const r = db().prepare('SELECT * FROM mfa_challenges WHERE id = ?').get(id) as Row | undefined;
  return r ? { id: r.id as string, userId: r.user_id as string, orgId: (r.org_id as string) ?? null, method: r.method as string, attempts: Number(r.attempts), expiresAt: r.expires_at as string } : null;
}

export type ChallengeResult = { ok: true; user: UserRecord; challenge: Challenge; usedRecoveryCode: boolean } | { ok: false; error: string; status: number };

/** Complete a challenge with a TOTP code or a recovery code (single use). */
export function completeChallenge(id: string, code: string): ChallengeResult {
  const ch = getChallenge(id);
  if (!ch || new Date(ch.expiresAt) <= new Date()) return { ok: false, error: 'The sign-in attempt expired. Sign in again.', status: 401 };
  if (ch.attempts >= MAX_ATTEMPTS) return { ok: false, error: 'Too many wrong codes. Sign in again.', status: 429 };
  const user = getUser(ch.userId);
  if (!user?.isActive || !user.totpSecretCiphertext) return { ok: false, error: 'Two-factor authentication is not set up', status: 400 };
  const step = verifyTotp(open(user.totpSecretCiphertext), code, user.lastTotpStep);
  if (step !== null) {
    updateUser(user.id, { lastTotpStep: step });
    db().prepare('DELETE FROM mfa_challenges WHERE id = ?').run(id);
    return { ok: true, user, challenge: ch, usedRecoveryCode: false };
  }
  const h = hashRecoveryCode(code);
  if (user.recoveryCodes.includes(h)) {
    updateUser(user.id, { recoveryCodes: user.recoveryCodes.filter((x) => x !== h) });
    db().prepare('DELETE FROM mfa_challenges WHERE id = ?').run(id);
    return { ok: true, user, challenge: ch, usedRecoveryCode: true };
  }
  db().prepare('UPDATE mfa_challenges SET attempts = attempts + 1 WHERE id = ?').run(id);
  return { ok: false, error: 'The code is not valid', status: 401 };
}

/** Start enrollment: a new secret kept as "pending" until confirmed with a code. */
export function beginEnrollment(user: UserRecord): string {
  const secret = newTotpSecret();
  updateUser(user.id, { totpPendingCiphertext: seal(secret) });
  return secret;
}

export function confirmEnrollment(user: UserRecord, code: string): { ok: true; recoveryCodes: string[] } | { ok: false; error: string } {
  if (!user.totpPendingCiphertext) return { ok: false, error: 'Start two-factor setup first' };
  const secret = open(user.totpPendingCiphertext);
  const step = verifyTotp(secret, code, null);
  if (step === null) return { ok: false, error: 'The code is not valid. Check the time on your device.' };
  const { codes, hashes } = newRecoveryCodes();
  updateUser(user.id, { totpSecretCiphertext: seal(secret), totpPendingCiphertext: null, mfaEnabled: true, lastTotpStep: step, recoveryCodes: hashes });
  return { ok: true, recoveryCodes: codes };
}

export function disableMfa(user: UserRecord): void {
  updateUser(user.id, { totpSecretCiphertext: null, totpPendingCiphertext: null, mfaEnabled: false, recoveryCodes: [], lastTotpStep: null });
}
