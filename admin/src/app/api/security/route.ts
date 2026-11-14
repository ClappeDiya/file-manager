import { NextRequest, NextResponse } from 'next/server';
import { audit, isContext, readJson, requirePermission, requireStepUp } from '@/lib/session';
import { getSecurity, setSecurity, type SecuritySettings } from '@/lib/db/repos/security';
import { orgErrorResponse } from '@/lib/route-helpers';

/** GET /api/security — the organization's security settings. */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'users.read');
  if (!isContext(ctx)) return ctx;
  return NextResponse.json({ settings: getSecurity(ctx.orgId) });
}

/** PUT /api/security {…settings} */
export async function PUT(request: NextRequest) {
  const ctx = await requireStepUp(request, 'sso.configure');
  if (!isContext(ctx)) return ctx;
  const b = (await readJson<Partial<SecuritySettings>>(request)) ?? {};
  try {
    const settings = setSecurity(ctx.orgId, b, ctx.user.email, ctx.ip);
    audit(ctx, { eventType: 'security.settings_update', severity: 'warning', description: 'Changed security settings', resourceType: 'organization', resourceId: ctx.orgId, metadata: { settings } });
    return NextResponse.json({ settings });
  } catch (e) {
    return orgErrorResponse(e);
  }
}
