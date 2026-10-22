import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { getManagedSettings, setManagedSettings } from '@/lib/db/repos/managed-settings';
import { listMachines } from '@/lib/db/repos/machines';
import { orgErrorResponse } from '@/lib/route-helpers';

const KEY = 'managed_settings';

/** GET /api/fleet/managed-settings — the settings and how many desktops applied them. */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'fleet.read');
  if (!isContext(ctx)) return ctx;
  const settings = getManagedSettings(ctx.orgId);
  const machines = listMachines({ orgIds: [ctx.orgId], status: 'active' }).filter((m) => m.agentKind === 'desktop');
  return NextResponse.json({
    settings,
    desktops: machines.length,
    upToDate: settings ? machines.filter((m) => m.appliedPolicies[KEY] === settings.version).length : 0,
  });
}

/** PUT /api/fleet/managed-settings {updates?, proxy?, ai?} — all null clears them. */
export async function PUT(request: NextRequest) {
  const ctx = await requirePermission(request, 'fleet.write');
  if (!isContext(ctx)) return ctx;
  const b = await readJson(request);
  if (!b) return badRequest('Invalid request');
  try {
    const s = setManagedSettings(ctx.orgId, b, ctx.user.email);
    audit(ctx, { eventType: 'fleet.managed_settings', description: s ? `Updated managed app settings (version ${s.version})` : 'Stopped managing app settings', resourceType: 'organization', resourceId: ctx.orgId, metadata: { settings: s } });
    return NextResponse.json({ settings: s });
  } catch (e) {
    return orgErrorResponse(e);
  }
}
