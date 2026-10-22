import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { createFleetPolicy, inheritedPolicies, listFleetPolicies, policiesFor, type FleetPolicy, type PolicyInput } from '@/lib/db/repos/fleet-policies';
import { listMachines, type Machine } from '@/lib/db/repos/machines';
import { managedSubtree } from '@/lib/db/repos/orgs';
import { orgErrorResponse } from '@/lib/route-helpers';

/**
 * GET /api/fleet/policies — this organization's policies and those inherited
 * from parents, with compliance (machines running it / up to date; inheriting
 * policies count machines of child organizations too).
 */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'fleet.read');
  if (!isContext(ctx)) return ctx;
  const machines = listMachines({ orgIds: managedSubtree(ctx.orgId), status: 'active' });
  const running = new Map<string, Machine[]>();
  for (const m of machines) for (const p of policiesFor(m)) running.set(p.id, [...(running.get(p.id) ?? []), m]);
  const compliance = (p: FleetPolicy) => {
    const targeted = (running.get(p.id) ?? []).filter((m) => p.inherit || m.orgId === ctx.orgId);
    return { targeted: targeted.length, upToDate: targeted.filter((m) => m.appliedPolicies[p.id] === p.version).length, pending: targeted.filter((m) => m.appliedPolicies[p.id] !== p.version).map((m) => ({ id: m.id, name: m.name, applied: m.appliedPolicies[p.id] ?? null })) };
  };
  return NextResponse.json({
    policies: listFleetPolicies(ctx.orgId).map((p) => ({ ...p, ...compliance(p) })),
    inherited: inheritedPolicies(ctx.orgId).map((i) => ({ ...i, ...compliance(i.policy) })),
  });
}

/** POST /api/fleet/policies {name, description?, target: {tags, machineIds, all}, job, inherit?, locked?} */
export async function POST(request: NextRequest) {
  const ctx = await requirePermission(request, 'fleet.write');
  if (!isContext(ctx)) return ctx;
  const b = await readJson<PolicyInput>(request);
  if (!b) return badRequest('Invalid request');
  try {
    const p = createFleetPolicy(ctx.orgId, b, ctx.user.email);
    audit(ctx, { eventType: 'fleet.policy_create', description: `Created fleet policy "${p.name}"`, resourceType: 'fleet_policy', resourceId: p.id });
    return NextResponse.json({ policy: p }, { status: 201 });
  } catch (e) {
    return orgErrorResponse(e);
  }
}
