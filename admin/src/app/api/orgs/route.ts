import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, canIn, isContext, readJson, requirePermission } from '@/lib/session';
import { accessibleOrgs, createOrg, license, OrgError, orgPath, setMembership } from '@/lib/db/repos/orgs';

/** GET /api/orgs — every organization the user can open, as a flat list with parents (build the tree client-side). */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'orgs.read');
  if (!isContext(ctx)) return ctx;
  const orgs = accessibleOrgs(ctx.user.id).map(({ org, role }) => ({ ...org, role, path: orgPath(org.id), license: license(org.id) }));
  return NextResponse.json({ organizations: orgs, currentOrgId: ctx.orgId });
}

/** POST /api/orgs {name, parentId?, detached?, licensePlan?, licenseSeats?} — create a sub-organization. */
export async function POST(request: NextRequest) {
  const ctx = await requirePermission(request, 'orgs.read');
  if (!isContext(ctx)) return ctx;
  const b = await readJson<{ name?: string; parentId?: string | null; detached?: boolean; licensePlan?: string | null; licenseSeats?: number | null }>(request);
  if (!b?.name) return badRequest('Give the organization a name');
  const parentId = b.parentId ?? ctx.orgId;
  if (!canIn(ctx, parentId, 'orgs.write')) return badRequest('You cannot create organizations there', 403);
  try {
    const org = createOrg({ name: b.name, parentId, detached: b.detached, licensePlan: b.licensePlan ?? null, licenseSeats: b.licenseSeats ?? null });
    // A detached tree is not managed from above, so its creator becomes its owner.
    if (org.detached) setMembership(ctx.user.id, org.id, 'org_owner');
    audit(ctx, { eventType: 'org.create', description: `Created organization "${org.name}" under ${orgPath(parentId)}`, resourceType: 'organization', resourceId: org.id, metadata: { detached: org.detached } });
    return NextResponse.json({ organization: org }, { status: 201 });
  } catch (e) {
    if (e instanceof OrgError) return badRequest(e.message, e.status);
    throw e;
  }
}
