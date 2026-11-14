import { NextRequest, NextResponse } from 'next/server';
import { can, isContext, requirePermission } from '@/lib/session';
import { CATALOG, listRuns, listSchedules } from '@/lib/report-runner';

/** GET /api/reports — the catalog (reports your role can run), schedules and recent runs. */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request);
  if (!isContext(ctx)) return ctx;
  const catalog = CATALOG().filter((r) => can(ctx, r.permission));
  const allowed = new Set(catalog.map((r) => r.id));
  return NextResponse.json({
    catalog,
    schedules: listSchedules(ctx.orgId).filter((s) => allowed.has(s.reportId)),
    runs: listRuns(ctx.orgId).filter((r) => allowed.has(r.reportId)),
  });
}
