import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requireStepUp } from '@/lib/session';
import { deleteProvider, updateProvider, type ProviderInput } from '@/lib/db/repos/sso';
import { orgErrorResponse } from '@/lib/route-helpers';

type Params = { params: Promise<{ id: string }> };

export async function PATCH(request: NextRequest, { params }: Params) {
  const ctx = await requireStepUp(request, 'sso.configure');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  const b = await readJson<Partial<ProviderInput>>(request);
  if (!b) return badRequest('Invalid request');
  try {
    const p = updateProvider(ctx.orgId, id, b);
    if (!p) return badRequest('Not found', 404);
    audit(ctx, { eventType: 'sso.provider_update', severity: 'warning', description: `Changed sign-in "${p.name}"${b.secret !== undefined ? ' (new secret)' : ''}`, resourceType: 'sso_provider', resourceId: id });
    return NextResponse.json({ provider: p });
  } catch (e) {
    return orgErrorResponse(e);
  }
}

export async function DELETE(request: NextRequest, { params }: Params) {
  const ctx = await requireStepUp(request, 'sso.configure');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  if (!deleteProvider(ctx.orgId, id)) return badRequest('Not found', 404);
  audit(ctx, { eventType: 'sso.provider_delete', severity: 'warning', description: `Removed sign-in provider ${id}`, resourceType: 'sso_provider', resourceId: id });
  return NextResponse.json({ ok: true });
}
