import { NextRequest, NextResponse } from 'next/server';
import { audit, isContext, readJson, requirePermission } from '@/lib/session';
import { setPolicyOptOut } from '@/lib/db/repos/fleet-policies';
import { orgErrorResponse } from '@/lib/route-helpers';

/** POST /api/fleet/policies/:id/opt-out {optOut: boolean} — for a policy inherited from a parent organization. */
export async function POST(request: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  const ctx = await requirePermission(request, 'fleet.write');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  const b = (await readJson<{ optOut?: boolean }>(request)) ?? {};
  try {
    const i = setPolicyOptOut(ctx.orgId, id, b.optOut !== false, ctx.user.email);
    audit(ctx, {
      eventType: 'fleet.policy_opt_out',
      severity: 'warning',
      description: `${i.optedOutHere ? 'Opted out of' : 'Opted back into'} "${i.policy.name}" inherited from ${i.sourceOrgName}`,
      resourceType: 'fleet_policy',
      resourceId: id,
    });
    return NextResponse.json({ inherited: i });
  } catch (e) {
    return orgErrorResponse(e);
  }
}
