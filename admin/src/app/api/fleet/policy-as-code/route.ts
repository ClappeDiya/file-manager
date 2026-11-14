import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { applyPolicies, exportPolicies } from '@/lib/policy-as-code';
import { orgErrorResponse } from '@/lib/route-helpers';

/** GET /api/fleet/policy-as-code — fleet policies, app settings and alert rules as one document. */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'fleet.read', 'alerts.read');
  if (!isContext(ctx)) return ctx;
  return NextResponse.json(exportPolicies(ctx.orgId), { headers: { 'Content-Disposition': 'inline; filename="ufop-policies.json"' } });
}

/** PUT /api/fleet/policy-as-code {document, prune?} — make the organization match the document (all or nothing). */
export async function PUT(request: NextRequest) {
  const ctx = await requirePermission(request, 'fleet.write', 'alerts.write');
  if (!isContext(ctx)) return ctx;
  const b = await readJson<{ document?: unknown; prune?: boolean }>(request);
  if (!b?.document) return badRequest('Send {document, prune?}');
  try {
    const plan = applyPolicies(ctx.orgId, b.document, ctx.user.email, !!b.prune);
    audit(ctx, {
      eventType: 'fleet.policy_as_code',
      description: `Applied a policy document: ${plan.summary.create} created, ${plan.summary.update} updated, ${plan.summary.delete} deleted`,
      resourceType: 'organization',
      resourceId: ctx.orgId,
      metadata: { changes: plan.changes.filter((c) => c.action !== 'unchanged'), prune: !!b.prune },
    });
    return NextResponse.json({ plan });
  } catch (e) {
    return orgErrorResponse(e);
  }
}
