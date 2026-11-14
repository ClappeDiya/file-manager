/**
 * "Why can this person do that?" — the effective-access explanation behind
 * Users → Explain: every role grant (membership, groups, ancestors) and
 * whether it applies, the resulting permissions, machine scoping from group
 * grants, and the organization rules that gate sign-in. Built on the same
 * functions that enforce access (`roleSources`, `ROLE_PERMISSIONS`,
 * `visibleMachineIds`' grants), so it cannot drift from them.
 */
import { db, type Row } from './db';
import { getOrg, highestRole, roleSources, type RoleSource } from './db/repos/orgs';
import { getSecurity, mfaRequired } from './db/repos/security';
import { hasPasskeys } from './db/repos/passkeys';
import { getUser } from './db/repos/users';
import { ROLE_PERMISSIONS, type Permission, type Role } from './types/auth';

export interface ScopedGrant {
  resourceType: string;
  resourceId: string;
  groupId: string;
  groupName: string;
}

export interface Condition {
  key: 'account_active' | 'membership_end' | 'two_factor' | 'networks';
  ok: boolean;
  detail: string;
}

export interface AccessExplanation {
  user: { id: string; email: string; name: string };
  org: { id: string; name: string };
  role: Role | null;
  sources: RoleSource[];
  permissions: Permission[];
  /** Only the `user` role is limited to granted machines. */
  scoped: boolean;
  grants: ScopedGrant[];
  conditions: Condition[];
  check: { permission: Permission; allowed: boolean; reason: string } | null;
}

export function explainAccess(userId: string, orgId: string, permission?: Permission): AccessExplanation | null {
  const user = getUser(userId);
  const org = getOrg(orgId);
  if (!user || !org) return null;
  const sources = roleSources(userId, orgId);
  const role = highestRole(sources);
  const permissions = role ? (ROLE_PERMISSIONS[role] ?? []) : [];
  const grants = (
    db()
      .prepare(
        'SELECT a.resource_type, a.resource_id, g.id AS group_id, g.name AS group_name FROM group_access a JOIN groups g ON g.id = a.group_id JOIN group_members m ON m.group_id = g.id WHERE m.user_id = ? AND g.org_id = ? ORDER BY g.name, a.resource_type, a.resource_id',
      )
      .all(userId, orgId) as Row[]
  ).map((r) => ({ resourceType: r.resource_type as string, resourceId: r.resource_id as string, groupId: r.group_id as string, groupName: r.group_name as string }));

  const sec = getSecurity(orgId);
  const end = db().prepare('SELECT expires_at FROM org_memberships WHERE user_id = ? AND org_id = ?').get(userId, orgId) as Row | undefined;
  const needs2fa = mfaRequired(user, false);
  const has2fa = user.mfaEnabled || hasPasskeys(userId);
  const conditions: Condition[] = [
    { key: 'account_active', ok: user.isActive, detail: user.isActive ? 'The account is active' : 'The account is deactivated: no access anywhere' },
    { key: 'membership_end', ok: true, detail: end?.expires_at ? `The direct membership ends ${end.expires_at as string}` : 'No membership end date' },
    {
      key: 'two_factor',
      ok: !needs2fa || has2fa,
      detail: needs2fa ? (has2fa ? 'Two-factor sign-in is required and set up' : 'Two-factor sign-in is required but not set up: the account can only set it up') : 'Two-factor sign-in is not required',
    },
    { key: 'networks', ok: true, detail: sec.ipAllowlist.length ? `Console access only from ${sec.ipAllowlist.join(', ')}` : 'Console access from any network' },
  ];

  let check: AccessExplanation['check'] = null;
  if (permission) {
    const winner = sources.find((x) => x.applies && x.role === role);
    const allowed = user.isActive && !!role && permissions.includes(permission);
    const reason = !user.isActive
      ? 'The account is deactivated'
      : !role
        ? 'No role applies in this organization'
        : permissions.includes(permission)
          ? `${role} includes ${permission} (role from ${winner ? describe(winner) : 'a grant'})`
          : `${role} does not include ${permission}`;
    check = { permission, allowed, reason };
  }
  return { user: { id: user.id, email: user.email, name: user.name }, org: { id: org.id, name: org.name }, role, sources, permissions, scoped: role === 'user', grants, conditions, check };
}

export function describe(s: RoleSource): string {
  const what = s.via === 'membership' ? 'direct membership' : `group "${s.groupName}"`;
  return `${what} in ${s.inherited ? `parent organization "${s.orgName}"` : `"${s.orgName}"`}`;
}
