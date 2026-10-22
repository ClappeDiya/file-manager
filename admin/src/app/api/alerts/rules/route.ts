import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { createRule, listRules, type RuleInput } from '@/lib/db/repos/alerts';
import { orgErrorResponse } from '@/lib/route-helpers';

export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'alerts.read');
  if (!isContext(ctx)) return ctx;
  return NextResponse.json({ rules: listRules(ctx.orgId) });
}

export async function POST(request: NextRequest) {
  const ctx = await requirePermission(request, 'alerts.write');
  if (!isContext(ctx)) return ctx;
  const b = await readJson<RuleInput>(request);
  if (!b) return badRequest('Invalid request');
  try {
    const r = createRule(ctx.orgId, b);
    audit(ctx, { eventType: 'alert.rule_create', description: `Created alert rule "${r.name}" (${r.kind})`, resourceType: 'alert_rule', resourceId: r.id });
    return NextResponse.json({ rule: r }, { status: 201 });
  } catch (e) {
    return orgErrorResponse(e);
  }
}
