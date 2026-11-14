import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission, requireStepUp } from '@/lib/session';
import { createScimToken, listScimTokens, SCIM_ROLES } from '@/lib/db/repos/scim';
import { orgErrorResponse, publicOrigin } from '@/lib/route-helpers';
import type { Role } from '@/lib/types/auth';

/** GET /api/scim-tokens — tokens of the organization (never the secrets) and the SCIM base URL. */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'users.write');
  if (!isContext(ctx)) return ctx;
  return NextResponse.json({ tokens: listScimTokens(ctx.orgId), baseUrl: `${publicOrigin(request)}/api/scim/v2`, roles: SCIM_ROLES });
}

/** POST /api/scim-tokens {name, defaultRole?} — the secret is returned once. */
export async function POST(request: NextRequest) {
  const ctx = await requireStepUp(request, 'users.write');
  if (!isContext(ctx)) return ctx;
  const b = await readJson<{ name?: string; defaultRole?: Role }>(request);
  if (!b) return badRequest('Invalid request');
  try {
    const { token, secret } = createScimToken(ctx.orgId, b.name ?? '', b.defaultRole ?? 'user', ctx.user.email);
    audit(ctx, { eventType: 'scim.token_create', severity: 'warning', description: `Created SCIM token "${token.name}" (default role ${token.defaultRole})`, resourceType: 'scim_token', resourceId: token.id });
    return NextResponse.json({ token, secret, baseUrl: `${publicOrigin(request)}/api/scim/v2` }, { status: 201 });
  } catch (e) {
    return orgErrorResponse(e);
  }
}
