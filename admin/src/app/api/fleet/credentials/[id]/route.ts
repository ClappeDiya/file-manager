import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { deleteCredential, rotateCredential } from '@/lib/db/repos/fleet-policies';
import { orgErrorResponse } from '@/lib/route-helpers';

type P = { params: Promise<{ id: string }> };

/** PUT {secret} — rotate; policies using it are re-versioned so agents pick it up. */
export async function PUT(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'credentials.write');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  const b = await readJson<{ secret?: string }>(request);
  try {
    if (!rotateCredential(ctx.orgId, id, b?.secret ?? '')) return badRequest('Credential not found', 404);
  } catch (e) {
    return orgErrorResponse(e);
  }
  audit(ctx, { eventType: 'credential.rotate', severity: 'warning', description: `Rotated central credential ${id}`, resourceType: 'credential', resourceId: id });
  return NextResponse.json({ success: true });
}

export async function DELETE(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'credentials.write');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  try {
    if (!deleteCredential(ctx.orgId, id)) return badRequest('Credential not found', 404);
  } catch (e) {
    return orgErrorResponse(e);
  }
  audit(ctx, { eventType: 'credential.delete', severity: 'warning', description: `Deleted central credential ${id}`, resourceType: 'credential', resourceId: id });
  return NextResponse.json({ success: true });
}
