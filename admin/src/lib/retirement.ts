/**
 * Retiring inactive access: an organization can set `retireInactiveDays`
 * (Settings → Security). Members whose last activity — sign-in, session use
 * or API token use, else when they joined — is older are removed from the
 * organization (and the account is disabled when it belongs to no other);
 * API tokens unused that long are revoked. Owners are never retired, so an
 * organization cannot lose its last administrator this way. Runs with the
 * periodic jobs; every change is audited.
 */
import { db, type Row } from './db';
import { listOrgs, membersOf, membershipsOf, removeMembership } from './db/repos/orgs';
import { getSecurity } from './db/repos/security';
import { getUser, updateUser } from './db/repos/users';
import { revokeUserSessions } from './db/repos/sessions';
import { listApiTokens, revokeApiToken, revokeUserApiTokens } from './db/repos/api-tokens';
import { appendAudit } from './db/repos/audit';

const latest = (...ts: (string | null | undefined)[]) => Math.max(0, ...ts.filter((t): t is string => !!t).map((t) => new Date(t).getTime()));

/** The most recent time the account did anything (sign-in, session, token), or when it joined. */
export function lastActivity(userId: string, joinedAt: string): number {
  const u = getUser(userId);
  const r = db().prepare('SELECT MAX(last_seen_at) AS s FROM sessions WHERE user_id = ?').get(userId) as Row;
  const t = db().prepare('SELECT MAX(last_used_at) AS t FROM api_tokens WHERE user_id = ?').get(userId) as Row;
  return latest(u?.lastLoginAt, joinedAt, r.s as string | null, t.t as string | null);
}

export interface RetirementResult {
  members: { orgId: string; userId: string; email: string; disabled: boolean }[];
  tokens: { orgId: string; id: string; name: string }[];
}

export function retireInactive(at = new Date()): RetirementResult {
  const out: RetirementResult = { members: [], tokens: [] };
  for (const org of listOrgs()) {
    const days = getSecurity(org.id).retireInactiveDays;
    if (!days) continue;
    const cutoff = at.getTime() - days * 86_400_000;
    for (const m of membersOf(org.id)) {
      if (m.role === 'org_owner' || lastActivity(m.userId, m.createdAt) >= cutoff) continue;
      const u = getUser(m.userId);
      if (!u) continue;
      removeMembership(m.userId, org.id);
      const disabled = membershipsOf(m.userId).length === 0;
      if (disabled) {
        updateUser(m.userId, { isActive: false });
        revokeUserSessions(m.userId, 'retired after inactivity');
        revokeUserApiTokens(m.userId);
      }
      out.members.push({ orgId: org.id, userId: m.userId, email: u.email, disabled });
      appendAudit({
        eventType: 'user.retired_inactive',
        severity: 'warning',
        orgId: org.id,
        userId: m.userId,
        userEmail: u.email,
        description: `Removed ${u.email} after ${days} days without activity${disabled ? ' (account disabled)' : ''}`,
        resourceType: 'user',
        resourceId: m.userId,
        metadata: { days },
      });
    }
    for (const t of listApiTokens(org.id, null)) {
      if (t.revokedAt || new Date(t.expiresAt).getTime() <= at.getTime() || latest(t.lastUsedAt, t.createdAt) >= cutoff) continue;
      if (!revokeApiToken(t.id)) continue;
      out.tokens.push({ orgId: org.id, id: t.id, name: t.name });
      appendAudit({
        eventType: 'api_token.retired_inactive',
        severity: 'warning',
        orgId: org.id,
        userId: t.userId,
        description: `Revoked API token "${t.name}" after ${days} days without use`,
        resourceType: 'api_token',
        resourceId: t.id,
        metadata: { days },
      });
    }
  }
  return out;
}
