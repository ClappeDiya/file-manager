import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission, requireStepUp } from '@/lib/session';
import { createProvider, listProviders, type ProviderInput } from '@/lib/db/repos/sso';
import { callbackUrl } from '@/lib/sso';
import { orgErrorResponse, publicOrigin } from '@/lib/route-helpers';

/** GET /api/sso — identity providers of the current organization (secrets never leave the server). */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'sso.configure');
  if (!isContext(ctx)) return ctx;
  const origin = publicOrigin(request);
  return NextResponse.json({
    providers: listProviders(ctx.orgId).map((p) => ({ ...p, callbackUrl: callbackUrl(origin, p.id), metadataUrl: p.kind === 'saml' ? `${origin}/api/auth/sso/${p.id}/metadata` : null })),
  });
}

export async function POST(request: NextRequest) {
  const ctx = await requireStepUp(request, 'sso.configure');
  if (!isContext(ctx)) return ctx;
  const b = await readJson<ProviderInput>(request);
  if (!b) return badRequest('Invalid request');
  try {
    const p = createProvider(ctx.orgId, b);
    audit(ctx, { eventType: 'sso.provider_create', severity: 'warning', description: `Added ${p.kind.toUpperCase()} sign-in "${p.name}"`, resourceType: 'sso_provider', resourceId: p.id });
    return NextResponse.json({ provider: p, callbackUrl: callbackUrl(publicOrigin(request), p.id) }, { status: 201 });
  } catch (e) {
    return orgErrorResponse(e);
  }
}
