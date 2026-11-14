import { NextRequest, NextResponse } from 'next/server';
import { publicOrigin } from '@/lib/route-helpers';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { manageableGroup } from '@/lib/group-access';
import { demoteCheck, removeGroupMember, setGroupMember, type GroupRole } from '@/lib/db/repos/groups';
import { effectiveRole, OrgError } from '@/lib/db/repos/orgs';
import { findUserByEmail, getUser } from '@/lib/db/repos/users';
import { createInvitation } from '@/lib/db/repos/invitations';
import { sendInvitationEmail } from '@/lib/mailer';

type P = { params: Promise<{ id: string }> };

/**
 * POST /api/groups/:id/members {userId | email, role} — add a member.
 * People without access to the organization get an invitation (role `user`)
 * that adds them to this group when accepted.
 */
export async function POST(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'groups.read');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  const m = manageableGroup(ctx, id);
  if (!m) return badRequest('Only the group\'s owners or administrators can add members', 403);
  const b = await readJson<{ userId?: string; email?: string; role?: GroupRole }>(request);
  const role: GroupRole = b?.role === 'owner' ? 'owner' : 'member';
  const user = b?.userId ? getUser(b.userId) : b?.email ? findUserByEmail(b.email) : null;
  try {
    if (user && effectiveRole(user.id, m.group.orgId)) {
      setGroupMember(id, user.id, role);
      audit(ctx, { eventType: 'group.member_add', description: `Added ${user.email} to "${m.group.name}" as ${role}`, resourceType: 'group', resourceId: id });
      return NextResponse.json({ added: user.id, role });
    }
    if (!b?.email) return badRequest('Enter the person\'s email address', 400);
    const { invitation, token } = createInvitation(m.group.orgId, { email: b.email, role: 'user', groupId: id, groupRole: role }, ctx.user.email);
    const link = `${publicOrigin(request)}/invite/${encodeURIComponent(token)}`;
    const sent = await sendInvitationEmail(m.group.orgId, invitation.email, link, ctx.user.name);
    audit(ctx, { eventType: 'group.invite', description: `Invited ${invitation.email} to "${m.group.name}" as ${role}`, resourceType: 'invitation', resourceId: invitation.id });
    return NextResponse.json({ invitation, link, emailed: sent.sent }, { status: 201 });
  } catch (e) {
    if (e instanceof OrgError) return badRequest(e.message, e.status);
    throw e;
  }
}

/** PATCH /api/groups/:id/members {userId, role} — change owner/member. */
export async function PATCH(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'groups.read');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  const m = manageableGroup(ctx, id);
  if (!m) return badRequest('Only the group\'s owners or administrators can change members', 403);
  const b = await readJson<{ userId?: string; role?: GroupRole }>(request);
  if (!b?.userId || !b.role || !['owner', 'member'].includes(b.role)) return badRequest('userId and role (owner or member) are required');
  try {
    demoteCheck(id, b.userId, b.role);
    setGroupMember(id, b.userId, b.role);
  } catch (e) {
    if (e instanceof OrgError) return badRequest(e.message, e.status);
    throw e;
  }
  audit(ctx, { eventType: 'group.member_role', description: `Set ${b.userId} to ${b.role} in "${m.group.name}"`, resourceType: 'group', resourceId: id });
  return NextResponse.json({ success: true });
}

/** DELETE /api/groups/:id/members?userId= */
export async function DELETE(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'groups.read');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  const userId = new URL(request.url).searchParams.get('userId');
  const m = manageableGroup(ctx, id);
  // Members may leave a group on their own.
  if (!m && userId !== ctx.user.id) return badRequest('Only the group\'s owners or administrators can remove members', 403);
  if (!userId) return badRequest('userId is required');
  try {
    if (!removeGroupMember(id, userId)) return badRequest('Not a member', 404);
  } catch (e) {
    if (e instanceof OrgError) return badRequest(e.message, e.status);
    throw e;
  }
  audit(ctx, { eventType: 'group.member_remove', description: `Removed ${userId} from group ${id}`, resourceType: 'group', resourceId: id });
  return NextResponse.json({ success: true });
}
