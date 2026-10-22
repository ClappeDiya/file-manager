import { NextRequest, NextResponse } from 'next/server';
import { revokeUserApiTokens } from '@/lib/db/repos/api-tokens';
import { publicOrigin } from '@/lib/route-helpers';
import { ROLE_HIERARCHY, type Role } from '@/lib/types/auth';
import { audit, badRequest, can, canIn, isContext, readJson, requirePermission, requireStepUp } from '@/lib/session';
import { canManageRole } from '@/lib/utils/permissions';
import { effectiveRole, membersOf, membershipsOf, OrgError, removeMembership, setMembership } from '@/lib/db/repos/orgs';
import { listUsers, publicUser, updateUser } from '@/lib/db/repos/users';
import { groupsOfUser } from '@/lib/db/repos/groups';
import { createInvitation, listInvitations } from '@/lib/db/repos/invitations';
import { revokeUserSessions } from '@/lib/db/repos/sessions';
import { sendInvitationEmail } from '@/lib/mailer';

/** GET /api/users — the organization's roster: members (with groups) and invitations. */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'users.read');
  if (!isContext(ctx)) return ctx;
  const { searchParams } = new URL(request.url);
  const role = searchParams.get('role');
  const search = searchParams.get('search')?.toLowerCase();
  const active = searchParams.get('active');
  const members = membersOf(ctx.orgId);
  const byId = new Map(members.map((m) => [m.userId, m]));
  let users = listUsers(members.map((m) => m.userId)).map((u) => ({
    ...publicUser(u),
    role: byId.get(u.id)!.role,
    memberSince: byId.get(u.id)!.createdAt,
    groups: groupsOfUser(u.id).filter((g) => g.orgId === ctx.orgId).map((g) => ({ id: g.id, name: g.name, role: g.memberRole })),
  }));
  if (role) users = users.filter((u) => u.role === role);
  if (active !== null) users = users.filter((u) => u.isActive === (active === 'true'));
  if (search) users = users.filter((u) => u.name.toLowerCase().includes(search) || u.email.toLowerCase().includes(search));
  const invitations = can(ctx, 'users.invite') ? listInvitations(ctx.orgId) : [];
  return NextResponse.json({ users, total: users.length, invitations });
}

/** POST /api/users — invite someone to this organization (optionally into a group). */
export async function POST(request: NextRequest) {
  const ctx = await requirePermission(request, 'users.invite');
  if (!isContext(ctx)) return ctx;
  const b = await readJson<{ email?: string; role?: Role; groupId?: string | null; groupRole?: 'owner' | 'member' }>(request);
  if (!b?.email) return badRequest('Email is required');
  const role: Role = b.role ?? 'user';
  if (!(role in ROLE_HIERARCHY)) return badRequest('Unknown role');
  if (role !== 'user' && !canManageRole(ctx.role, role) && ctx.role !== 'org_owner') return badRequest('You cannot invite someone with a role at or above your own', 403);
  try {
    const { invitation, token } = createInvitation(ctx.orgId, { email: b.email, role, groupId: b.groupId ?? null, groupRole: b.groupRole ?? 'member' }, ctx.user.email);
    const link = `${publicOrigin(request)}/invite/${encodeURIComponent(token)}`;
    const delivery = await sendInvitationEmail(ctx.orgId, invitation.email, link, ctx.user.name);
    audit(ctx, { eventType: 'admin.user_invite', description: `Invited ${invitation.email} as ${role}`, resourceType: 'invitation', resourceId: invitation.id, metadata: { emailed: delivery.sent } });
    return NextResponse.json({ invitation, link, emailed: delivery.sent, emailError: delivery.error }, { status: 201 });
  } catch (e) {
    if (e instanceof OrgError) return badRequest(e.message, e.status);
    throw e;
  }
}

/** PATCH /api/users — bulk actions on members of this organization. */
export async function PATCH(request: NextRequest) {
  const b = await readJson<{ action?: string; userIds?: string[]; role?: Role }>(request);
  const perm = b?.action === 'remove' ? 'users.remove' : b?.action === 'set_role' ? 'roles.write' : 'users.deactivate'; // deactivate, activate, sign_out
  // Removing members and changing roles are sensitive: they need a recent sign-in.
  const ctx = b?.action === 'remove' || b?.action === 'set_role' ? await requireStepUp(request, perm) : await requirePermission(request, perm);
  if (!isContext(ctx)) return ctx;
  if (!b?.action || !Array.isArray(b.userIds) || b.userIds.length === 0) return badRequest('Action and userIds required');
  if (b.userIds.includes(ctx.user.id)) return badRequest('Cannot modify your own account via bulk operations', 403);
  const members = new Map(membersOf(ctx.orgId).map((m) => [m.userId, m]));
  for (const id of b.userIds) {
    const m = members.get(id);
    if (!m) return badRequest(`User ${id} is not a member of this organization`, 404);
    // Compare with the effective role here (a parent's role is delegated down), not just the direct membership.
    const theirs = effectiveRole(id, ctx.orgId) ?? m.role;
    if (ctx.role !== 'org_owner' && !canManageRole(ctx.role, theirs)) return badRequest('You cannot change someone whose role is at or above your own', 403);
    // Deactivation locks the whole account: only when every organization it belongs to is yours to manage.
    if ((b.action === 'deactivate' || b.action === 'activate' || b.action === 'sign_out') && membershipsOf(id).some((x) => x.orgId !== ctx.orgId && !canIn(ctx, x.orgId, 'users.deactivate'))) {
      return badRequest('This account also belongs to other organizations; remove it from this organization instead', 403);
    }
  }
  const owners = [...members.values()].filter((m) => m.role === 'org_owner' && !b.userIds!.includes(m.userId));
  switch (b.action) {
    case 'deactivate':
    case 'activate':
      for (const id of b.userIds) {
        updateUser(id, { isActive: b.action === 'activate' });
        if (b.action === 'deactivate') {
          revokeUserSessions(id, 'deactivated');
          revokeUserApiTokens(id);
        }
      }
      break;
    case 'sign_out':
      for (const id of b.userIds) revokeUserSessions(id, `signed out by ${ctx.user.email}`);
      break;
    case 'remove':
      if (owners.length === 0) return badRequest('Cannot remove the last organization owner', 403);
      for (const id of b.userIds) removeMembership(id, ctx.orgId);
      break;
    case 'set_role': {
      if (!b.role || !(b.role in ROLE_HIERARCHY)) return badRequest('Unknown role');
      if (ctx.role !== 'org_owner' && !canManageRole(ctx.role, b.role)) return badRequest('You cannot assign a role at or above your own', 403);
      if (b.role !== 'org_owner' && owners.length === 0) return badRequest('Cannot demote the last organization owner', 403);
      for (const id of b.userIds) setMembership(id, ctx.orgId, b.role);
      break;
    }
    default:
      return badRequest('Unknown action');
  }
  const event = { deactivate: 'admin.user_deactivate', activate: 'admin.user_activate', remove: 'admin.user_remove', set_role: 'admin.role_change', sign_out: 'admin.user_sign_out' }[b.action] ?? 'admin.settings_change';
  audit(ctx, { eventType: event, description: `${b.action} ${b.userIds.length} user(s)${b.role ? ` → ${b.role}` : ''}`, metadata: { userIds: b.userIds } });
  return NextResponse.json({ success: true, affected: b.userIds.length });
}
