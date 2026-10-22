/**
 * Invitations: a one-time link (token stored hashed) that adds a person to an
 * organization with a role, and optionally to a group. Accepting creates the
 * account (password or SSO) or attaches an existing one.
 */
import { db, hashToken, newId, newToken, now, tx, type Row } from '../index';
import { ROLE_HIERARCHY, type Role } from '../../types/auth';
import { getOrg, OrgError, setMembership } from './orgs';
import { createUser, findUserByEmail, type UserRecord } from './users';
import { getGroup, setGroupMember, type GroupRole } from './groups';

export const INVITE_DAYS = 7;

export interface Invitation {
  id: string;
  orgId: string;
  email: string;
  role: Role;
  groupId: string | null;
  groupRole: GroupRole;
  invitedBy: string;
  createdAt: string;
  expiresAt: string;
  acceptedAt: string | null;
  acceptedUserId: string | null;
  revokedAt: string | null;
  status: 'pending' | 'accepted' | 'revoked' | 'expired';
}

function fromRow(r: Row): Invitation {
  const status = r.accepted_at ? 'accepted' : r.revoked_at ? 'revoked' : new Date(r.expires_at as string) <= new Date() ? 'expired' : 'pending';
  return {
    id: r.id as string,
    orgId: r.org_id as string,
    email: r.email as string,
    role: r.role as Role,
    groupId: (r.group_id as string) ?? null,
    groupRole: r.group_role as GroupRole,
    invitedBy: r.invited_by as string,
    createdAt: r.created_at as string,
    expiresAt: r.expires_at as string,
    acceptedAt: (r.accepted_at as string) ?? null,
    acceptedUserId: (r.accepted_user_id as string) ?? null,
    revokedAt: (r.revoked_at as string) ?? null,
    status,
  };
}

export function listInvitations(orgId: string): Invitation[] {
  return (db().prepare('SELECT * FROM invitations WHERE org_id = ? ORDER BY created_at DESC').all(orgId) as Row[]).map(fromRow);
}

export function getInvitation(id: string): Invitation | null {
  const r = db().prepare('SELECT * FROM invitations WHERE id = ?').get(id) as Row | undefined;
  return r ? fromRow(r) : null;
}

export interface NewInvitation {
  email: string;
  role: Role;
  groupId?: string | null;
  groupRole?: GroupRole;
}

/** Returns the invitation and the raw token (shown once, sent in the link). */
export function createInvitation(orgId: string, i: NewInvitation, invitedBy: string): { invitation: Invitation; token: string } {
  const email = i.email?.trim().toLowerCase();
  if (!email || !/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(email)) throw new OrgError('Enter a valid email address');
  if (i.groupId) {
    const g = getGroup(i.groupId);
    if (!g || g.orgId !== orgId) throw new OrgError('The group is not in this organization', 404);
  }
  const existing = findUserByEmail(email);
  if (existing && db().prepare('SELECT 1 FROM org_memberships WHERE user_id = ? AND org_id = ?').get(existing.id, orgId) && !i.groupId) {
    throw new OrgError('This person is already a member', 409);
  }
  const token = newToken('ufop_inv');
  const id = newId('inv');
  const ts = now();
  // A new invitation replaces an older pending one for the same address.
  db().prepare('UPDATE invitations SET revoked_at = ? WHERE org_id = ? AND lower(email) = ? AND accepted_at IS NULL AND revoked_at IS NULL').run(ts, orgId, email);
  db()
    .prepare('INSERT INTO invitations (id, org_id, email, role, group_id, group_role, token_hash, invited_by, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)')
    .run(id, orgId, email, i.role, i.groupId ?? null, i.groupRole ?? 'member', hashToken(token), invitedBy, ts, new Date(Date.now() + INVITE_DAYS * 86_400_000).toISOString());
  return { invitation: getInvitation(id)!, token };
}

export function revokeInvitation(orgId: string, id: string): boolean {
  return db().prepare('UPDATE invitations SET revoked_at = ? WHERE org_id = ? AND id = ? AND accepted_at IS NULL AND revoked_at IS NULL').run(now(), orgId, id).changes > 0;
}

export function findByToken(token: string): Invitation | null {
  const r = db().prepare('SELECT * FROM invitations WHERE token_hash = ?').get(hashToken(token)) as Row | undefined;
  return r ? fromRow(r) : null;
}

/**
 * Accept: `account` is either an existing signed-in user (must match the
 * invited address) or the details for a new account.
 */
export function acceptInvitation(token: string, account: { existing: UserRecord } | { name: string; passwordHash: string | null; ssoProvider?: 'saml' | 'scim' | null }): { user: UserRecord; invitation: Invitation } {
  const inv = findByToken(token);
  if (!inv) throw new OrgError('This invitation link is not valid', 404);
  if (inv.status !== 'pending') throw new OrgError(`This invitation is ${inv.status}`, 410);
  if (!getOrg(inv.orgId)) throw new OrgError('The organization no longer exists', 410);
  return tx(() => {
    let user: UserRecord;
    if ('existing' in account) {
      if (account.existing.email.toLowerCase() !== inv.email) throw new OrgError('This invitation is for a different email address', 403);
      user = account.existing;
    } else {
      if (findUserByEmail(inv.email)) throw new OrgError('An account already exists for this address. Sign in, then open the link again.', 409);
      user = createUser({ email: inv.email, name: account.name, role: inv.role, organizationId: inv.orgId, passwordHash: account.passwordHash, ssoProvider: account.ssoProvider ?? null });
    }
    // Never lower an existing membership (e.g. a member invited into a group).
    const cur = db().prepare('SELECT role FROM org_memberships WHERE user_id = ? AND org_id = ?').get(user.id, inv.orgId) as Row | undefined;
    if (!cur || (ROLE_HIERARCHY[inv.role] ?? 0) > (ROLE_HIERARCHY[cur.role as Role] ?? 0)) setMembership(user.id, inv.orgId, inv.role);
    if (inv.groupId && getGroup(inv.groupId)) setGroupMember(inv.groupId, user.id, inv.groupRole);
    db().prepare('UPDATE invitations SET accepted_at = ?, accepted_user_id = ? WHERE id = ?').run(now(), user.id, inv.id);
    return { user, invitation: getInvitation(inv.id)! };
  });
}
