import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { createFleetPolicy, listFleetPolicies, policyApplies, type PolicyInput } from '@/lib/db/repos/fleet-policies';
import { listMachines } from '@/lib/db/repos/machines';
import { orgErrorResponse } from '@/lib/route-helpers';

/** GET /api/fleet/policies — with per-policy compliance (machines targeted / up to date). */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'fleet.read');
  if (!isContext(ctx)) return ctx;
  const machines = listMachines({ orgIds: [ctx.orgId], status: 'active' });
  return NextResponse.json({
    policies: listFleetPolicies(ctx.orgId).map((p) => {
      const targeted = machines.filter((m) => policyApplies(p, m));
      return { ...p, targeted: targeted.length, upToDate: targeted.filter((m) => m.appliedPolicies[p.id] === p.version).length, pending: targeted.filter((m) => m.appliedPolicies[p.id] !== p.version).map((m) => ({ id: m.id, name: m.name, applied: m.appliedPolicies[p.id] ?? null })) };
    }),
  });
}

/** POST /api/fleet/policies {name, description?, target: {tags, machineIds, all}, job} */
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
