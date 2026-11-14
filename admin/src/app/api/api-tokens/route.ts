import { NextRequest, NextResponse } from 'next/server';
import { audit, can, isContext, readJson, requirePermission, requireStepUp } from '@/lib/session';
import { createApiToken, listApiTokens, MAX_DAYS } from '@/lib/db/repos/api-tokens';
import { orgErrorResponse } from '@/lib/route-helpers';

/**
 * GET /api/api-tokens — your API tokens in this organization (never the secrets);
 * `?all=1` with users.write: every member's. Also the permissions you can grant.
 */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request);
  if (!isContext(ctx)) return ctx;
  const all = new URL(request.url).searchParams.get('all') === '1' && can(ctx, 'users.write');
  return NextResponse.json({
    tokens: listApiTokens(ctx.orgId, all ? null : ctx.user.id),
    grantable: ctx.permissions,
    maxDays: MAX_DAYS,
  });
}

/** POST /api/api-tokens {name, permissions[], expiresInDays} — the secret is returned once. */
export async function POST(request: NextRequest) {
  const ctx = await requireStepUp(request);
  if (!isContext(ctx)) return ctx;
  const b =
    (await readJson<{
      name?: string;
      permissions?: unknown;
      expiresInDays?: unknown;
    }>(request)) ?? {};
  try {
    const { token, secret } = createApiToken({
      orgId: ctx.orgId,
      userId: ctx.user.id,
      name: b.name ?? '',
      permissions: b.permissions,
      days: b.expiresInDays ?? 90,
      allowed: ctx.permissions,
    });
    audit(ctx, {
      eventType: 'api_token.create',
      severity: 'warning',
      description: `Created API token "${token.name}" (${token.permissions.length} permissions, expires ${token.expiresAt.slice(0, 10)})`,
      resourceType: 'api_token',
      resourceId: token.id,
      metadata: { permissions: token.permissions },
    });
    return NextResponse.json({ token, secret }, { status: 201 });
  } catch (e) {
    return orgErrorResponse(e);
  }
}
