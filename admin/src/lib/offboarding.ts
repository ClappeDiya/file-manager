/**
 * Removing people from an organization, reversibly.
 *
 * `offboardMember` is the one path for every removal (the Users page,
 * membership expiry, inactivity retirement; callers audit): it keeps what is needed to put
 * the member back for `RESTORE_DAYS`, leaves the organization's groups,
 * revokes the member's API tokens there, and hands over what they own to a
 * successor when one is named (group ownership, alert email recipients). An
 * account that belongs to no organization afterwards is disabled and signed
 * out. `restoreMember` undoes it within the window; afterwards
 * `purgeRemovedMembers` forgets the record and deletes accounts left with
 * nothing.
 */
import { db, now, parseJson, tx, type Row } from './db';
import { getOrg, membershipsOf, setMembership } from './db/repos/orgs';
import { getUser, updateUser, deleteUser } from './db/repos/users';
import { revokeUserSessions } from './db/repos/sessions';
import { listApiTokens, revokeApiToken } from './db/repos/api-tokens';
import { listChannels, type EmailConfig } from './db/repos/channels';
import { appendAudit } from './db/repos/audit';
import { OrgError } from './db/repos/orgs';
import type { Role } from './types/auth';

export const RESTORE_DAYS = 30;

export type RemovalReason = 'removed' | 'expired' | 'inactive';

export interface RemovedMember {
  userId: string;
  orgId: string;
  email: string;
  name: string;
  role: Role;
  reason: RemovalReason;
  removedBy: string;
  removedAt: string;
  /** After this the member can no longer be restored. */
  purgeAfter: string;
  accountDisabled: boolean;
  reassignedTo: string | null;
}

export interface OffboardResult {
  groupsLeft: number;
  groupsHandedOver: number;
  tokensRevoked: number;
  channelsUpdated: number;
  accountDisabled: boolean;
}

interface GroupSeat {
  id: string;
  role: string;
}

/**
 * Remove a member from an organization. `successor` (a member of the same
 * organization) takes over group ownership and alert email recipients.
 */
export function offboardMember(orgId: string, userId: string, o: { by: string; reason: RemovalReason; successor?: string | null }): OffboardResult {
  const user = getUser(userId);
  const m = db().prepare('SELECT * FROM org_memberships WHERE user_id = ? AND org_id = ?').get(userId, orgId) as Row | undefined;
  if (!user || !m) throw new OrgError('Not a member of this organization', 404);
  const successor = o.successor ? getUser(o.successor) : null;
  if (o.successor && (!successor || successor.id === userId || !db().prepare('SELECT 1 FROM org_memberships WHERE user_id = ? AND org_id = ?').get(o.successor, orgId))) {
    throw new OrgError('The successor must be another member of this organization');
  }
  return tx(() => {
    const res: OffboardResult = { groupsLeft: 0, groupsHandedOver: 0, tokensRevoked: 0, channelsUpdated: 0, accountDisabled: false };
    // Groups of this organization.
    const seats = db().prepare('SELECT m.group_id AS id, m.role FROM group_members m JOIN groups g ON g.id = m.group_id WHERE m.user_id = ? AND g.org_id = ?').all(userId, orgId) as GroupSeat[];
    for (const s of seats) {
      const owners = (db().prepare("SELECT COUNT(*) AS n FROM group_members WHERE group_id = ? AND role = 'owner'").get(s.id) as { n: number }).n;
      if (s.role === 'owner' && owners === 1 && successor) {
        db().prepare("INSERT INTO group_members (group_id, user_id, role, added_at) VALUES (?, ?, 'owner', ?) ON CONFLICT(group_id, user_id) DO UPDATE SET role = 'owner'").run(s.id, successor.id, now());
        res.groupsHandedOver++;
      }
      db().prepare('DELETE FROM group_members WHERE group_id = ? AND user_id = ?').run(s.id, userId);
      res.groupsLeft++;
    }
    // API tokens for this organization.
    for (const t of listApiTokens(orgId, userId)) if (revokeApiToken(t.id)) res.tokensRevoked++;
    // Alert emails: hand over to the successor, or drop the address where others remain.
    for (const c of listChannels(orgId)) {
      if (c.kind !== 'email') continue;
      const cfg = c.config as EmailConfig;
      const to = cfg.to ?? [];
      if (!to.some((x) => x.toLowerCase() === user.email.toLowerCase())) continue;
      let next = to.filter((x) => x.toLowerCase() !== user.email.toLowerCase());
      if (successor && !next.some((x) => x.toLowerCase() === successor.email.toLowerCase())) next = [...next, successor.email];
      if (!next.length) continue; // the only recipient and no successor: left for an administrator
      db()
        .prepare('UPDATE notification_channels SET config = ? WHERE id = ?')
        .run(JSON.stringify({ ...cfg, to: next }), c.id);
      res.channelsUpdated++;
    }
    db().prepare('DELETE FROM org_memberships WHERE user_id = ? AND org_id = ?').run(userId, orgId);
    if (membershipsOf(userId).length === 0 && user.isActive) {
      updateUser(userId, { isActive: false });
      revokeUserSessions(userId, `removed from ${getOrg(orgId)?.name ?? orgId}`);
      res.accountDisabled = true;
    }
    const ts = now();
    db()
      .prepare(
        `INSERT INTO removed_members (user_id, org_id, role, group_ids, reason, removed_by, removed_at, purge_after, account_disabled, reassigned_to) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(user_id, org_id) DO UPDATE SET role = excluded.role, group_ids = excluded.group_ids, reason = excluded.reason, removed_by = excluded.removed_by, removed_at = excluded.removed_at, purge_after = excluded.purge_after, account_disabled = excluded.account_disabled, reassigned_to = excluded.reassigned_to`,
      )
      .run(userId, orgId, m.role, JSON.stringify(seats), o.reason, o.by, ts, new Date(Date.now() + RESTORE_DAYS * 86_400_000).toISOString(), res.accountDisabled ? 1 : 0, successor?.id ?? null);
    return res;
  });
}

function removedFromRow(r: Row): RemovedMember {
  return {
    userId: r.user_id as string,
    orgId: r.org_id as string,
    email: (r.email as string) ?? '',
    name: (r.name as string) ?? '',
    role: r.role as Role,
    reason: r.reason as RemovalReason,
    removedBy: r.removed_by as string,
    removedAt: r.removed_at as string,
    purgeAfter: r.purge_after as string,
    accountDisabled: !!r.account_disabled,
    reassignedTo: (r.reassigned_to as string) ?? null,
  };
}

/** Members removed from an organization who can still be restored. */
export function listRemovedMembers(orgId: string): RemovedMember[] {
  return (db().prepare('SELECT r.*, u.email, u.name FROM removed_members r JOIN users u ON u.id = r.user_id WHERE r.org_id = ? AND r.purge_after > ? ORDER BY r.removed_at DESC').all(orgId, now()) as Row[]).map(
    removedFromRow,
  );
}

/** Put a removed member back with their role and groups (those that still exist). */
export function restoreMember(orgId: string, userId: string): RemovedMember {
  const r = db().prepare('SELECT r.*, u.email, u.name FROM removed_members r JOIN users u ON u.id = r.user_id WHERE r.user_id = ? AND r.org_id = ?').get(userId, orgId) as Row | undefined;
  if (!r || new Date(r.purge_after as string) <= new Date()) throw new OrgError('Nothing to restore: the member was not removed or the restore window has passed', 404);
  const rec = removedFromRow(r);
  return tx(() => {
    setMembership(userId, orgId, rec.role);
    for (const s of parseJson<GroupSeat[]>(r.group_ids, [])) {
      if (db().prepare('SELECT 1 FROM groups WHERE id = ? AND org_id = ?').get(s.id, orgId)) {
        db().prepare('INSERT OR IGNORE INTO group_members (group_id, user_id, role, added_at) VALUES (?, ?, ?, ?)').run(s.id, userId, s.role, now());
      }
    }
    if (rec.accountDisabled) updateUser(userId, { isActive: true });
    db().prepare('DELETE FROM removed_members WHERE user_id = ? AND org_id = ?').run(userId, orgId);
    return rec;
  });
}

/** Set (or clear, with null) when a membership ends. */
export function setMembershipExpiry(orgId: string, userId: string, expiresAt: string | null): void {
  if (expiresAt !== null && (Number.isNaN(Date.parse(expiresAt)) || Date.parse(expiresAt) <= Date.now())) throw new OrgError('The expiry must be a future date');
  const ok =
    db()
      .prepare('UPDATE org_memberships SET expires_at = ? WHERE user_id = ? AND org_id = ?')
      .run(expiresAt ? new Date(expiresAt).toISOString() : null, userId, orgId).changes > 0;
  if (!ok) throw new OrgError('Not a member of this organization', 404);
}

export function membershipExpiry(orgId: string, userId: string): string | null {
  const r = db().prepare('SELECT expires_at FROM org_memberships WHERE user_id = ? AND org_id = ?').get(userId, orgId) as Row | undefined;
  return (r?.expires_at as string) ?? null;
}

/** Remove members whose membership has expired (owners included: an expiry is explicit). */
export function expireMemberships(at = new Date()): { orgId: string; userId: string }[] {
  const due = db().prepare('SELECT user_id, org_id, role FROM org_memberships WHERE expires_at IS NOT NULL AND expires_at <= ?').all(at.toISOString()) as Row[];
  const out: { orgId: string; userId: string }[] = [];
  for (const r of due) {
    const orgId = r.org_id as string;
    // Never leave an organization without an owner.
    if (r.role === 'org_owner' && (db().prepare("SELECT COUNT(*) AS n FROM org_memberships WHERE org_id = ? AND role = 'org_owner'").get(orgId) as { n: number }).n === 1) continue;
    const res = offboardMember(orgId, r.user_id as string, { by: 'system', reason: 'expired' });
    appendAudit({
      eventType: 'user.membership_expired',
      severity: 'warning',
      orgId,
      userId: r.user_id as string,
      description: `Membership of ${getUser(r.user_id as string)?.email ?? r.user_id} expired${res.accountDisabled ? '; account disabled' : ''}`,
      resourceType: 'user',
      resourceId: r.user_id as string,
      metadata: { ...res },
    });
    out.push({ orgId, userId: r.user_id as string });
  }
  return out;
}

/** Forget removals past the restore window; delete accounts left with no organization. */
export function purgeRemovedMembers(at = new Date()): string[] {
  const gone = db().prepare('SELECT user_id FROM removed_members WHERE purge_after <= ?').all(at.toISOString()) as Row[];
  db().prepare('DELETE FROM removed_members WHERE purge_after <= ?').run(at.toISOString());
  const deleted: string[] = [];
  for (const id of new Set(gone.map((r) => r.user_id as string))) {
    const u = getUser(id);
    if (!u || u.isActive || membershipsOf(id).length || db().prepare('SELECT 1 FROM removed_members WHERE user_id = ?').get(id)) continue;
    if (deleteUser(id)) {
      deleted.push(id);
      appendAudit({
        eventType: 'admin.user_purge',
        severity: 'warning',
        userEmail: 'system',
        description: `Deleted the account ${u.email} ${RESTORE_DAYS} days after its last removal`,
        resourceType: 'user',
        resourceId: id,
      });
    }
  }
  return deleted;
}
