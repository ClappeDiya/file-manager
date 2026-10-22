import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission, requireStepUp } from '@/lib/session';
import { credentialHistory, deleteCredential, rotateCredential } from '@/lib/db/repos/fleet-policies';
import { orgErrorResponse } from '@/lib/route-helpers';

type P = { params: Promise<{ id: string }> };

/** GET — rotation history (versions, who, when; never secrets). */
export async function GET(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'fleet.read');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  const history = credentialHistory(ctx.orgId, id);
  if (!history.length) return badRequest('Credential not found', 404);
  return NextResponse.json({ history });
}

/** PUT {secret} — rotate: a new version that agents fetch on their next heartbeat. */
export async function PUT(request: NextRequest, { params }: P) {
  const ctx = await requireStepUp(request, 'credentials.write');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  const b = await readJson<{ secret?: string }>(request);
  let c;
  try {
    c = rotateCredential(ctx.orgId, id, b?.secret ?? '', ctx.user.email);
    if (!c) return badRequest('Credential not found', 404);
  } catch (e) {
    return orgErrorResponse(e);
  }
  audit(ctx, { eventType: 'credential.rotate', severity: 'warning', description: `Rotated central credential "${c.name}" to version ${c.version}`, resourceType: 'credential', resourceId: id, metadata: { version: c.version } });
  return NextResponse.json({ success: true, credential: c });
}

export async function DELETE(request: NextRequest, { params }: P) {
  const ctx = await requireStepUp(request, 'credentials.write');
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
