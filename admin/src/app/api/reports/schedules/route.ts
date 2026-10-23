import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, can, isContext, readJson, requirePermission } from '@/lib/session';
import { reportDefinition } from '@/lib/report-catalog';
import { createSchedule, type ScheduleInput } from '@/lib/report-runner';
import { orgErrorResponse } from '@/lib/route-helpers';

/** POST /api/reports/schedules {reportId, frequency, hour, weekday?, dayOfMonth?, timeZone?, days?, format?, channelIds, name?} */
export async function POST(request: NextRequest) {
  const ctx = await requirePermission(request, 'alerts.write');
  if (!isContext(ctx)) return ctx;
  const b = (await readJson<ScheduleInput>(request)) ?? ({} as ScheduleInput);
  const def = reportDefinition(b.reportId);
  if (def && !can(ctx, def.permission)) return badRequest(`Forbidden: requires ${def.permission}`, 403);
  try {
    const s = createSchedule(ctx.orgId, b, ctx.user.email);
    audit(ctx, { eventType: 'report.schedule_create', description: `Scheduled report "${s.name}" (${s.frequency} at ${s.hour}:00 ${s.timeZone})`, resourceType: 'report_schedule', resourceId: s.id });
    return NextResponse.json({ schedule: s }, { status: 201 });
  } catch (e) {
    return orgErrorResponse(e);
  }
}
