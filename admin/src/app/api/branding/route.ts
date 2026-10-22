import { NextRequest, NextResponse } from 'next/server';
import { audit, authenticate, can, isContext, readJson, requirePermission } from '@/lib/session';
import { DEFAULT_BRANDING, EMAIL_TEMPLATES, effectiveBranding, ownBranding, setBranding } from '@/lib/branding';
import { orgErrorResponse } from '@/lib/route-helpers';

/** GET /api/branding — the branding in force for the current organization (any member), its own settings and the template defaults. */
export async function GET(request: NextRequest) {
  const ctx = await authenticate(request);
  if (!isContext(ctx)) return ctx;
  const canEdit = can(ctx, 'orgs.write');
  return NextResponse.json({
    branding: effectiveBranding(ctx.orgId),
    ...(canEdit ? { own: ownBranding(ctx.orgId), defaults: DEFAULT_BRANDING, templates: EMAIL_TEMPLATES } : {}),
    canEdit,
  });
}

/** PUT /api/branding {displayName, primaryColor, logo, emailFromName, emailFooter, supportEmail, supportUrl, templates} */
export async function PUT(request: NextRequest) {
  const ctx = await requirePermission(request, 'orgs.write');
  if (!isContext(ctx)) return ctx;
  try {
    const b = setBranding(ctx.orgId, (await readJson(request)) ?? {}, ctx.user.email);
    audit(ctx, {
      eventType: 'branding.update',
      description: `Changed the organization's branding (${b.displayName})`,
      resourceType: 'organization',
      resourceId: ctx.orgId,
      metadata: { templates: Object.keys(b.templates), logo: !!b.logo, color: b.primaryColor },
    });
    return NextResponse.json({ branding: b });
  } catch (e) {
    return orgErrorResponse(e);
  }
}

/** DELETE /api/branding — remove this organization's branding (it inherits again). */
export async function DELETE(request: NextRequest) {
  const ctx = await requirePermission(request, 'orgs.write');
  if (!isContext(ctx)) return ctx;
  const b = setBranding(ctx.orgId, null, ctx.user.email);
  audit(ctx, { eventType: 'branding.reset', description: "Removed the organization's own branding", resourceType: 'organization', resourceId: ctx.orgId });
  return NextResponse.json({ branding: b });
}
