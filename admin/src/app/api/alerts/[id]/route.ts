import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, requirePermission } from '@/lib/session';
import { getAlert, resolveAlert } from '@/lib/db/repos/alerts';

/** PATCH /api/alerts/:id — mark resolved by hand. */
export async function PATCH(request: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  const ctx = await requirePermission(request, 'alerts.write');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  const a = getAlert(ctx.orgId, id);
  if (!a) return badRequest('Alert not found', 404);
  resolveAlert(id);
  audit(ctx, { eventType: 'alert.resolve', description: `Resolved: ${a.message}`, resourceType: 'alert', resourceId: id });
  return NextResponse.json({ alert: getAlert(ctx.orgId, id) });
}
