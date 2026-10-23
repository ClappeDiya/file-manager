/** Stored passkeys (WebAuthn credentials) and one-time ceremony challenges. */
import { db, newId, now, parseJson, type Row } from '../index';
import { newChallenge } from '../../webauthn';
import { OrgError } from './orgs';

export interface Passkey {
  credentialId: string;
  userId: string;
  name: string;
  publicKey: string;
  alg: number;
  signCount: number;
  transports: string[];
  aaguid: string;
  createdAt: string;
  lastUsedAt: string | null;
}

/** The public shape (no key material). */
export type PasskeyInfo = Omit<Passkey, 'publicKey'>;

function fromRow(r: Row): Passkey {
  return {
    credentialId: r.credential_id as string,
    userId: r.user_id as string,
    name: r.name as string,
    publicKey: r.public_key as string,
    alg: Number(r.alg),
    signCount: Number(r.sign_count),
    transports: parseJson<string[]>(r.transports, []),
    aaguid: r.aaguid as string,
    createdAt: r.created_at as string,
    lastUsedAt: (r.last_used_at as string) ?? null,
  };
}

export const MAX_PASSKEYS = 20;

export function listPasskeys(userId: string): Passkey[] {
  return (db().prepare('SELECT * FROM passkeys WHERE user_id = ? ORDER BY created_at').all(userId) as Row[]).map(fromRow);
}

export function publicPasskey(p: Passkey): PasskeyInfo {
  const { publicKey: _k, ...rest } = p;
  void _k;
  return rest;
}

export function hasPasskeys(userId: string): boolean {
  return !!db().prepare('SELECT 1 FROM passkeys WHERE user_id = ? LIMIT 1').get(userId);
}

export function getPasskey(credentialId: string): Passkey | null {
  const r = db().prepare('SELECT * FROM passkeys WHERE credential_id = ?').get(credentialId) as Row | undefined;
  return r ? fromRow(r) : null;
}

export function addPasskey(p: Omit<Passkey, 'createdAt' | 'lastUsedAt'>): Passkey {
  if (listPasskeys(p.userId).length >= MAX_PASSKEYS) throw new OrgError(`At most ${MAX_PASSKEYS} passkeys per account`);
  if (getPasskey(p.credentialId)) throw new OrgError('This passkey is already registered', 409);
  db()
    .prepare('INSERT INTO passkeys (credential_id, user_id, name, public_key, alg, sign_count, transports, aaguid, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)')
    .run(p.credentialId, p.userId, p.name.trim().slice(0, 80) || 'Passkey', p.publicKey, p.alg, p.signCount, JSON.stringify(p.transports.slice(0, 8)), p.aaguid, now());
  return getPasskey(p.credentialId)!;
}

export function touchPasskey(credentialId: string, signCount: number): void {
  db().prepare('UPDATE passkeys SET sign_count = ?, last_used_at = ? WHERE credential_id = ?').run(signCount, now(), credentialId);
}

export function renamePasskey(userId: string, credentialId: string, name: string): boolean {
  return db().prepare('UPDATE passkeys SET name = ? WHERE user_id = ? AND credential_id = ?').run(name.trim().slice(0, 80) || 'Passkey', userId, credentialId).changes > 0;
}

export function deletePasskey(userId: string, credentialId: string): boolean {
  return db().prepare('DELETE FROM passkeys WHERE user_id = ? AND credential_id = ?').run(userId, credentialId).changes > 0;
}

export type CeremonyPurpose = 'register' | 'login' | 'reauth';
const CHALLENGE_MINUTES = 5;

export function createCeremony(purpose: CeremonyPurpose, userId: string | null): { id: string; challenge: string } {
  const id = newId('wac');
  const challenge = newChallenge();
  db()
    .prepare('INSERT INTO webauthn_challenges (id, user_id, purpose, challenge, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?)')
    .run(id, userId, purpose, challenge, now(), new Date(Date.now() + CHALLENGE_MINUTES * 60_000).toISOString());
  db().prepare('DELETE FROM webauthn_challenges WHERE expires_at < ?').run(new Date(Date.now() - 3600_000).toISOString());
  return { id, challenge };
}

/** Use a challenge once; null when unknown, used, expired or for another purpose/user. */
export function consumeCeremony(id: string, purpose: CeremonyPurpose, userId: string | null): string | null {
  const r = db().prepare('SELECT * FROM webauthn_challenges WHERE id = ?').get(id) as Row | undefined;
  if (!r || r.used_at || r.purpose !== purpose || new Date(r.expires_at as string) <= new Date() || (userId !== null && r.user_id !== userId)) return null;
  const ok = db().prepare('UPDATE webauthn_challenges SET used_at = ? WHERE id = ? AND used_at IS NULL').run(now(), id).changes === 1;
  return ok ? (r.challenge as string) : null;
}
