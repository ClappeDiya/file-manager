import { NextRequest, NextResponse } from 'next/server';
import { audit, authenticate, badRequest, isContext, readJson, requireStepUp } from '@/lib/session';
import { getUser, publicUser } from '@/lib/db/repos/users';
import { effectiveRole } from '@/lib/db/repos/orgs';
import { startViewAs, stopViewAs } from '@/lib/db/repos/sessions';
import { outranks } from '@/lib/utils/permissions';

const DEFAULT_MINUTES = 30;

/** GET /api/view-as — whether this session is viewing the console as someone. */
export async function GET(request: NextRequest) {
  const ctx = await authenticate(request);
  if (!isContext(ctx)) return ctx;
  return NextResponse.json({ viewAs: ctx.viewAs ? { user: publicUser(ctx.user), role: ctx.role, actor: publicUser(ctx.viewAs.actor), until: ctx.viewAs.until } : null });
}

/**
 * POST /api/view-as {userId, minutes?} — view the console as a member of
 * this organization, read-only, for 5–60 minutes (default 30). Needs user
 * management, a recent sign-in, and a role at least as high as theirs.
 */
export async function POST(request: NextRequest) {
  const ctx = await requireStepUp(request, 'users.write');
  if (!isContext(ctx)) return ctx;
  if (ctx.viewAs) return badRequest('Stop viewing as the current member first');
  const b = await readJson<{ userId?: string; minutes?: number }>(request);
  const target = b?.userId ? getUser(b.userId) : null;
  if (!target || !target.isActive) return NextResponse.json({ error: 'No such active member' }, { status: 404 });
  if (target.id === ctx.user.id) return badRequest('You are already yourself');
  const role = effectiveRole(target.id, ctx.orgId);
  if (!role) return NextResponse.json({ error: 'That user is not a member of this organization' }, { status: 404 });
  if (outranks(role, ctx.role)) return NextResponse.json({ error: 'You cannot view the console as someone with a higher role' }, { status: 403 });
  const minutes = Math.min(60, Math.max(5, Math.round(b?.minutes ?? DEFAULT_MINUTES)));
  const until = new Date(Date.now() + minutes * 60_000);
  startViewAs(ctx.sid, target.id, until);
  audit(ctx, { eventType: 'user.view_as_start', description: `Started viewing the console as ${target.email} (${role}, read-only, ${minutes} minutes)`, resourceType: 'user', resourceId: target.id, metadata: { until: until.toISOString(), role } });
  return NextResponse.json({ viewAs: { user: publicUser(target), role, actor: publicUser(ctx.user), until: until.toISOString() } });
}

/** DELETE /api/view-as — back to your own view. */
export async function DELETE(request: NextRequest) {
  const ctx = await authenticate(request);
  if (!isContext(ctx)) return ctx;
  if (ctx.viewAs) {
    stopViewAs(ctx.sid);
    audit(ctx, { eventType: 'user.view_as_stop', description: `Stopped viewing the console as ${ctx.user.email}`, resourceType: 'user', resourceId: ctx.user.id });
  }
  return NextResponse.json({ viewAs: null });
}
