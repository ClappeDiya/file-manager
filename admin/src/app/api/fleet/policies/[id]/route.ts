import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { deleteFleetPolicy, updateFleetPolicy, type PolicyInput } from '@/lib/db/repos/fleet-policies';
import { orgErrorResponse } from '@/lib/route-helpers';

type P = { params: Promise<{ id: string }> };

export async function PUT(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'fleet.write');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  const b = await readJson<Partial<PolicyInput>>(request);
  if (!b) return badRequest('Invalid request');
  try {
    const p = updateFleetPolicy(ctx.orgId, id, b);
    if (!p) return badRequest('Policy not found', 404);
    audit(ctx, { eventType: 'fleet.policy_update', description: `Updated fleet policy "${p.name}" to version ${p.version}`, resourceType: 'fleet_policy', resourceId: id });
    return NextResponse.json({ policy: p });
  } catch (e) {
    return orgErrorResponse(e);
  }
}

/** DELETE — agents remove the managed job on their next policy sync. */
export async function DELETE(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'fleet.write');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  if (!deleteFleetPolicy(ctx.orgId, id)) return badRequest('Policy not found', 404);
  audit(ctx, { eventType: 'fleet.policy_delete', severity: 'warning', description: `Deleted fleet policy ${id}`, resourceType: 'fleet_policy', resourceId: id });
  return NextResponse.json({ success: true });
}
