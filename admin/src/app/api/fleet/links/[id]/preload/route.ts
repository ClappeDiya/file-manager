import { NextRequest, NextResponse } from 'next/server';
import { publicOrigin } from '@/lib/route-helpers';
import { audit, badRequest, isContext, requirePermission } from '@/lib/session';
import { getLink, linkToken } from '@/lib/db/repos/machines';

/**
 * GET /api/fleet/links/:id/preload — a preload.json for unattended installs.
 * Placed at the platform's preload path (e.g. /etc/ufop/preload.json, not
 * writable by other users), it makes the desktop app, ufop-server and the CLI
 * register with this console on first start.
 */
export async function GET(request: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  const ctx = await requirePermission(request, 'fleet.enroll');
  if (!isContext(ctx)) return ctx;
  const { id } = await params;
  const link = getLink(ctx.orgId, id);
  if (!link) return badRequest('Link not found', 404);
  if (!link.usable) return badRequest('This link can no longer register machines', 410);
  const token = linkToken(ctx.orgId, id)!;
  const preload = {
    env: {
      '*': {
        UFOP_CONSOLE_URL: publicOrigin(request),
        UFOP_CONSOLE_REGISTRATION_TOKEN: token,
        ...(link.defaultTags.length ? { UFOP_CONSOLE_TAGS: link.defaultTags.join(',') } : {}),
      },
    },
    args: {},
    db: {},
  };
  audit(ctx, { eventType: 'fleet.preload_download', description: `Downloaded the enrollment preload for link "${link.label || link.tokenPrefix}"`, resourceType: 'registration_link', resourceId: id });
  return new NextResponse(JSON.stringify(preload, null, 2), { headers: { 'content-type': 'application/json', 'content-disposition': 'attachment; filename="preload.json"', 'cache-control': 'no-store' } });
}
