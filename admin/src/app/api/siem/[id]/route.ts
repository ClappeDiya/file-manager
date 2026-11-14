import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requireStepUp } from '@/lib/session';
import { deleteSiemDestination, siemBacklog, updateSiemDestination, type SiemInput } from '@/lib/db/repos/siem';
import { orgErrorResponse } from '@/lib/route-helpers';

type P = { params: Promise<{ id: string }> };

/** PATCH /api/siem/:id {name?, config?, secret?, includeChildren?, enabled?} */
export async function PATCH(request: NextRequest, { params }: P) {
  const ctx = await requireStepUp(request, 'audit.export');
  if (!isContext(ctx)) return ctx;
  const b = (await readJson<Partial<SiemInput>>(request)) ?? {};
  try {
    const d = updateSiemDestination(ctx.orgId, (await params).id, b);
    if (!d) return badRequest('Destination not found', 404);
    audit(ctx, {
      eventType: 'siem.destination_update',
      severity: 'warning',
      description: `Changed SIEM destination "${d.name}"${b.enabled === false ? ' (paused)' : ''}`,
      resourceType: 'siem_destination',
      resourceId: d.id,
    });
    return NextResponse.json({ destination: { ...d, backlog: siemBacklog(d) } });
  } catch (e) {
    return orgErrorResponse(e);
  }
}

export async function DELETE(request: NextRequest, { params }: P) {
  const ctx = await requireStepUp(request, 'audit.export');
  if (!isContext(ctx)) return ctx;
  const id = (await params).id;
  if (!deleteSiemDestination(ctx.orgId, id)) return badRequest('Destination not found', 404);
  audit(ctx, { eventType: 'siem.destination_delete', severity: 'warning', description: `Removed SIEM destination ${id}`, resourceType: 'siem_destination', resourceId: id });
  return NextResponse.json({ success: true });
}
