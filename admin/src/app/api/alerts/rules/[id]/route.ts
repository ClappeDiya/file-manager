import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { deleteRule, updateRule, type RuleInput } from '@/lib/db/repos/alerts';
import { orgErrorResponse } from '@/lib/route-helpers';

type P = { params: Promise<{ id: string }> };

export async function PUT(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'alerts.write');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  const b = await readJson<Partial<RuleInput>>(request);
  if (!b) return badRequest('Invalid request');
  try {
    const r = updateRule(ctx.orgId, id, b);
    if (!r) return badRequest('Rule not found', 404);
    audit(ctx, { eventType: 'alert.rule_update', description: `Updated alert rule "${r.name}"`, resourceType: 'alert_rule', resourceId: id });
    return NextResponse.json({ rule: r });
  } catch (e) {
    return orgErrorResponse(e);
  }
}

export async function DELETE(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'alerts.write');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  if (!deleteRule(ctx.orgId, id)) return badRequest('Rule not found', 404);
  audit(ctx, { eventType: 'alert.rule_delete', description: `Deleted alert rule ${id}`, resourceType: 'alert_rule', resourceId: id });
  return NextResponse.json({ success: true });
}
