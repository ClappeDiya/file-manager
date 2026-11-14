import { NextRequest, NextResponse } from 'next/server';
import { badRequest, can, isContext, requirePermission } from '@/lib/session';
import { reportDefinition } from '@/lib/report-catalog';
import { getSchedule, runSchedule } from '@/lib/report-runner';

/** POST — run the schedule now and send it to its channels (its regular times are unchanged). */
export async function POST(request: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  const ctx = await requirePermission(request, 'alerts.write');
  if (!isContext(ctx)) return ctx;
  const s = getSchedule(ctx.orgId, (await params).id);
  if (!s || !can(ctx, reportDefinition(s.reportId)?.permission ?? 'admin.access')) return badRequest('Schedule not found', 404);
  const run = await runSchedule(s, ctx.user.email);
  return NextResponse.json({ run }, { status: run.status === 'failed' ? 500 : 201 });
}
