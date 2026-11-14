import { NextRequest, NextResponse } from 'next/server';
import { paged } from '@/lib/pagination';
import { pagedJson, readPage } from '@/lib/route-helpers';
import { isContext, requirePermission } from '@/lib/session';
import { listAlerts } from '@/lib/db/repos/alerts';
import { visibleMachineIds } from '@/lib/scope';

/** GET /api/alerts?state=open|resolved|all */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'alerts.read');
  if (!isContext(ctx)) return ctx;
  const state = new URL(request.url).searchParams.get('state') ?? 'open';
  const pr = readPage(request, { defaultLimit: Infinity });
  if (pr instanceof NextResponse) return pr;
  const { items: alerts, page } = paged(listAlerts({ orgIds: [ctx.orgId], open: state === 'all' ? undefined : state === 'open', machineIds: visibleMachineIds(ctx) }), pr);
  return pagedJson(request, { alerts }, page);
}
