import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, can, isContext, readJson, requirePermission } from '@/lib/session';
import { reportDefinition } from '@/lib/report-catalog';
import { deleteSchedule, getSchedule, listRuns, updateSchedule, type ScheduleInput } from '@/lib/report-runner';
import { orgErrorResponse } from '@/lib/route-helpers';

type P = { params: Promise<{ id: string }> };

/** GET — the schedule and its runs. */
export async function GET(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'alerts.read');
  if (!isContext(ctx)) return ctx;
  const s = getSchedule(ctx.orgId, (await params).id);
  if (!s || !can(ctx, reportDefinition(s.reportId)?.permission ?? 'admin.access')) return badRequest('Schedule not found', 404);
  return NextResponse.json({ schedule: s, runs: listRuns(ctx.orgId, { scheduleId: s.id }) });
}

export async function PATCH(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'alerts.write');
  if (!isContext(ctx)) return ctx;
  const b = (await readJson<Partial<ScheduleInput>>(request)) ?? {};
  const id = (await params).id;
  const target = reportDefinition(b.reportId ?? getSchedule(ctx.orgId, id)?.reportId ?? '');
  if (target && !can(ctx, target.permission)) return badRequest(`Forbidden: requires ${target.permission}`, 403);
  try {
    const s = updateSchedule(ctx.orgId, id, b);
    if (!s) return badRequest('Schedule not found', 404);
    audit(ctx, { eventType: 'report.schedule_update', description: `Changed report schedule "${s.name}"${s.enabled ? '' : ' (paused)'}`, resourceType: 'report_schedule', resourceId: s.id });
    return NextResponse.json({ schedule: s });
  } catch (e) {
    return orgErrorResponse(e);
  }
}

export async function DELETE(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'alerts.write');
  if (!isContext(ctx)) return ctx;
  const id = (await params).id;
  if (!deleteSchedule(ctx.orgId, id)) return badRequest('Schedule not found', 404);
  audit(ctx, { eventType: 'report.schedule_delete', description: `Deleted report schedule ${id}`, resourceType: 'report_schedule', resourceId: id });
  return NextResponse.json({ success: true });
}
