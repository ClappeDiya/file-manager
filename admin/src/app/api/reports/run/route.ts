import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, can, isContext, readJson, requirePermission } from '@/lib/session';
import { reportDefinition } from '@/lib/report-catalog';
import { runReport } from '@/lib/report-runner';

/** POST /api/reports/run {reportId, days?} — run a report now; returns the run and its rows. */
export async function POST(request: NextRequest) {
  const ctx = await requirePermission(request);
  if (!isContext(ctx)) return ctx;
  const b = (await readJson<{ reportId?: string; days?: number }>(request)) ?? {};
  const def = b.reportId ? reportDefinition(b.reportId) : null;
  if (!def) return badRequest('Unknown report', 404);
  if (!can(ctx, def.permission)) return badRequest(`Forbidden: requires ${def.permission}`, 403);
  const { run, result } = runReport(ctx.orgId, def.id, { days: b.days, by: ctx.user.email });
  audit(ctx, { eventType: 'report.run', description: `Ran report "${def.name}" (${run.status}, ${run.rowCount} rows)`, resourceType: 'report_run', resourceId: run.id });
  return NextResponse.json({ run, result }, { status: run.status === 'failed' ? 500 : 201 });
}
