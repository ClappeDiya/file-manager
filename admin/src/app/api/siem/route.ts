import { NextRequest, NextResponse } from 'next/server';
import { audit, isContext, readJson, requirePermission, requireStepUp } from '@/lib/session';
import { createSiemDestination, listSiemDestinations, siemBacklog, SIEM_KINDS, type SiemInput } from '@/lib/db/repos/siem';
import { orgErrorResponse } from '@/lib/route-helpers';

/** GET /api/siem — destinations with their backlog (never the secrets). */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'audit.read');
  if (!isContext(ctx)) return ctx;
  return NextResponse.json({ destinations: listSiemDestinations(ctx.orgId).map((d) => ({ ...d, backlog: siemBacklog(d) })), kinds: SIEM_KINDS, jsonlAvailable: !!process.env.SIEM_JSONL_DIR });
}

/** POST /api/siem {name, kind, config, secret?, includeChildren?, backfill?} */
export async function POST(request: NextRequest) {
  const ctx = await requireStepUp(request, 'audit.export');
  if (!isContext(ctx)) return ctx;
  const b = (await readJson<SiemInput>(request)) ?? ({} as SiemInput);
  try {
    const d = createSiemDestination(ctx.orgId, b, ctx.user.email);
    audit(ctx, {
      eventType: 'siem.destination_create',
      severity: 'warning',
      description: `Added SIEM destination "${d.name}" (${d.kind}${d.includeChildren ? ', with child organizations' : ''}${b.backfill ? ', with history' : ''})`,
      resourceType: 'siem_destination',
      resourceId: d.id,
    });
    return NextResponse.json({ destination: d }, { status: 201 });
  } catch (e) {
    return orgErrorResponse(e);
  }
}
