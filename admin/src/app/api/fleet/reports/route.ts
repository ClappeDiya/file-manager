import { NextRequest, NextResponse } from 'next/server';
import { isContext, requirePermission } from '@/lib/session';
import { listReports, reportRaw } from '@/lib/db/repos/reports';
import { visibleMachineIds } from '@/lib/scope';

/** GET /api/fleet/reports?machineId=&backupId=&status=&days=&limit= | ?id= (raw report) */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'fleet.read');
  if (!isContext(ctx)) return ctx;
  const sp = new URL(request.url).searchParams;
  const ids = visibleMachineIds(ctx);
  const id = sp.get('id');
  if (id) {
    const [r] = listReports({ orgIds: [ctx.orgId], machineIds: ids, limit: 5000 }).filter((x) => x.id === id);
    return r ? NextResponse.json({ report: r, raw: reportRaw(id) }) : NextResponse.json({ error: 'Report not found' }, { status: 404 });
  }
  const days = Number(sp.get('days') ?? 0);
  return NextResponse.json({
    reports: listReports({ orgIds: [ctx.orgId], machineIds: ids, machineId: sp.get('machineId') || undefined, backupId: sp.get('backupId') || undefined, status: sp.get('status') || undefined, since: days ? new Date(Date.now() - days * 86_400_000).toISOString() : undefined, limit: Number(sp.get('limit') ?? 100) }),
  });
}
