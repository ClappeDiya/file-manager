import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { createLink, listLinks } from '@/lib/db/repos/machines';

/** GET /api/fleet/links — registration links (tokens are not listed). */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'fleet.enroll');
  if (!isContext(ctx)) return ctx;
  return NextResponse.json({ links: listLinks(ctx.orgId) });
}

/** POST /api/fleet/links {label, expiresInHours?, maxUses?, defaultTags?} → the token once. */
export async function POST(request: NextRequest) {
  const ctx = await requirePermission(request, 'fleet.enroll');
  if (!isContext(ctx)) return ctx;
  const b = await readJson<{ label?: string; expiresInHours?: number | null; maxUses?: number | null; defaultTags?: string[] }>(request);
  if (!b) return badRequest('Invalid request');
  if (b.maxUses !== undefined && b.maxUses !== null && (!Number.isInteger(b.maxUses) || b.maxUses < 1)) return badRequest('maxUses must be a positive whole number');
  const { link, token } = createLink(ctx.orgId, b, ctx.user.email);
  audit(ctx, { eventType: 'fleet.link_create', description: `Created registration link "${link.label || link.tokenPrefix}"`, resourceType: 'registration_link', resourceId: link.id });
  const origin = new URL(request.url).origin;
  return NextResponse.json({ link, token, consoleUrl: origin, reportUrl: `${origin}/api/ingest/${encodeURIComponent(token)}` }, { status: 201 });
}
