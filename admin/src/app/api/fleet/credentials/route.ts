import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { createCredential, credentialPropagation, listCredentials } from '@/lib/db/repos/fleet-policies';
import { listMachines } from '@/lib/db/repos/machines';
import { orgErrorResponse } from '@/lib/route-helpers';

/** GET /api/fleet/credentials — names and metadata only; secrets never leave except to agents. */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'fleet.read');
  if (!isContext(ctx)) return ctx;
  const propagation = credentialPropagation(ctx.orgId, listMachines({ orgIds: [ctx.orgId], status: 'active' }));
  return NextResponse.json({ credentials: listCredentials(ctx.orgId).map((c) => ({ ...c, propagation: propagation[c.id] ?? { targeted: 0, upToDate: 0, pending: [] } })) });
}

/** POST /api/fleet/credentials {name, kind?, description?, secret} */
export async function POST(request: NextRequest) {
  const ctx = await requirePermission(request, 'credentials.write');
  if (!isContext(ctx)) return ctx;
  const b = await readJson<{ name?: string; kind?: string; description?: string; secret?: string }>(request);
  if (!b) return badRequest('Invalid request');
  try {
    const c = createCredential(ctx.orgId, { name: b.name ?? '', kind: b.kind, description: b.description, secret: b.secret ?? '' }, ctx.user.email);
    audit(ctx, { eventType: 'credential.create', description: `Stored central credential "${c.name}"`, resourceType: 'credential', resourceId: c.id });
    return NextResponse.json({ credential: c }, { status: 201 });
  } catch (e) {
    return orgErrorResponse(e);
  }
}
