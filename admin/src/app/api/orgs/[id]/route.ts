import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, canIn, isContext, readJson, requirePermission } from '@/lib/session';
import { cancelOffboarding, childrenOf, effectiveRole, getOrg, license, membersOf, OrgError, orgPath, startOffboarding, updateOrg, type OrgUpdate } from '@/lib/db/repos/orgs';

type P = { params: Promise<{ id: string }> };

function guard(e: unknown) {
  if (e instanceof OrgError) return badRequest(e.message, e.status);
  throw e;
}

/** GET /api/orgs/:id — details, license and direct children. */
export async function GET(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'orgs.read');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  const org = getOrg(id);
  if (!org || !effectiveRole(ctx.user.id, id)) return badRequest('Organization not found', 404);
  return NextResponse.json({ organization: { ...org, path: orgPath(id), license: license(id), memberCount: membersOf(id).length }, children: childrenOf(id).filter((c) => effectiveRole(ctx.user.id, c.id)) });
}

/** PATCH /api/orgs/:id — rename, move, detach, protect, set license. */
export async function PATCH(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'orgs.read');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  if (!canIn(ctx, id, 'orgs.write')) return badRequest('You cannot change this organization', 403);
  const b = await readJson<OrgUpdate & { action?: string }>(request);
  if (!b) return badRequest('Invalid request');
  if (b.action === 'cancel_offboarding') {
    const org = cancelOffboarding(id);
    audit(ctx, { eventType: 'org.offboard_cancel', severity: 'warning', description: `Cancelled offboarding of "${org.name}"`, resourceType: 'organization', resourceId: id });
    return NextResponse.json({ organization: org });
  }
  const cur = getOrg(id)!;
  // Protection and detachment change who can delete/manage the tenant: owners only.
  if ((b.protected !== undefined || b.detached !== undefined) && effectiveRole(ctx.user.id, id) !== 'org_owner') return badRequest('Only organization owners can change protection or detachment', 403);
  // Licenses are granted from above: the parent's managers set a child's license.
  if ((b.licensePlan !== undefined || b.licenseSeats !== undefined) && !(cur.parentId && canIn(ctx, cur.parentId, 'orgs.write')) && id !== 'org_default') return badRequest('A license is set by the parent organization', 403);
  if (b.parentId !== undefined && b.parentId !== null && !canIn(ctx, b.parentId, 'orgs.write')) return badRequest('You cannot move organizations there', 403);
  if (id === 'org_default' && b.parentId) return badRequest('The default organization stays at the top');
  try {
    const upd: OrgUpdate = {};
    for (const k of ['name', 'parentId', 'detached', 'protected', 'licensePlan', 'licenseSeats'] as const) if (k in b) (upd as Record<string, unknown>)[k] = b[k];
    const org = updateOrg(id, upd);
    audit(ctx, { eventType: 'org.update', severity: b.detached !== undefined || b.protected !== undefined ? 'warning' : 'info', description: `Updated organization "${org.name}"`, resourceType: 'organization', resourceId: id, metadata: { fields: Object.keys(upd) } });
    return NextResponse.json({ organization: org });
  } catch (e) {
    return guard(e);
  }
}

/** DELETE /api/orgs/:id {confirmName, graceDays} — protected offboarding (access revoked now, data deleted after the grace period). */
export async function DELETE(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'orgs.read');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  if (effectiveRole(ctx.user.id, id) !== 'org_owner') return badRequest('Only organization owners can offboard an organization', 403);
  if (id === ctx.orgId) return badRequest('Switch to another organization before offboarding this one', 409);
  const b = await readJson<{ confirmName?: string; graceDays?: number }>(request);
  try {
    const org = startOffboarding(id, Math.min(90, Math.max(0, Number(b?.graceDays ?? 30))), b?.confirmName ?? '');
    audit(ctx, { eventType: 'org.offboard', severity: 'critical', description: `Started offboarding "${org.name}"; data is deleted after ${org.offboardAfter}`, resourceType: 'organization', resourceId: id });
    return NextResponse.json({ organization: org });
  } catch (e) {
    return guard(e);
  }
}
