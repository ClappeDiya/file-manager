import { NextRequest, NextResponse } from 'next/server';
import { badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { planPolicies } from '@/lib/policy-as-code';
import { orgErrorResponse } from '@/lib/route-helpers';

/** POST /api/fleet/policy-as-code/plan {document, prune?} — what applying would change; writes nothing. */
export async function POST(request: NextRequest) {
  const ctx = await requirePermission(request, 'fleet.read', 'alerts.read');
  if (!isContext(ctx)) return ctx;
  const b = await readJson<{ document?: unknown; prune?: boolean }>(request);
  if (!b?.document) return badRequest('Send {document, prune?}');
  try {
    return NextResponse.json({ plan: planPolicies(ctx.orgId, b.document, !!b.prune) });
  } catch (e) {
    return orgErrorResponse(e);
  }
}
