import { NextRequest, NextResponse } from 'next/server';
import { audit, isContext, readJson, requirePermission } from '@/lib/session';
import { analyze, listAiReports } from '@/lib/ai';
import { orgErrorResponse } from '@/lib/route-helpers';
import { getMachine } from '@/lib/db/repos/machines';
import { canSeeMachine } from '@/lib/scope';

/** GET /api/ai/analyze — recent analyses and summaries. */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'ai.read');
  if (!isContext(ctx)) return ctx;
  return NextResponse.json({ reports: listAiReports(ctx.orgId) });
}

/** POST /api/ai/analyze {machineId?, question?, days?} — run an analysis now. */
export async function POST(request: NextRequest) {
  const ctx = await requirePermission(request, 'ai.read', 'fleet.read');
  if (!isContext(ctx)) return ctx;
  const b = (await readJson<{ machineId?: string; question?: string; days?: number }>(request)) ?? {};
  if (b.machineId && !canSeeMachine(ctx, getMachine(b.machineId))) return NextResponse.json({ error: 'Machine not found' }, { status: 404 });
  if (ctx.role === 'user') return NextResponse.json({ error: 'Your role cannot run fleet analyses' }, { status: 403 });
  try {
    const r = await analyze(ctx.orgId, { kind: 'analysis', machineId: b.machineId, question: b.question, days: Math.min(90, Math.max(1, b.days ?? 7)), by: ctx.user.email });
    audit(ctx, { eventType: 'ai.analysis', description: r.error ? `AI analysis failed: ${r.error}` : 'Ran an AI backup analysis', resourceType: 'ai_report', resourceId: r.id, severity: r.error ? 'warning' : 'info' });
    return NextResponse.json({ report: r }, { status: r.error ? 502 : 201 });
  } catch (e) {
    return orgErrorResponse(e);
  }
}
