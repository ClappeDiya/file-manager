import { NextRequest, NextResponse } from 'next/server';
import { paged } from '@/lib/pagination';
import { pagedJson, readPage } from '@/lib/route-helpers';
import { isContext, requirePermission } from '@/lib/session';
import { listMachines, tagColors, type Connectivity, type MachineStatus } from '@/lib/db/repos/machines';
import { listBackups } from '@/lib/db/repos/reports';
import { listAlerts } from '@/lib/db/repos/alerts';
import { policiesFor } from '@/lib/db/repos/fleet-policies';
import { visibleMachineIds } from '@/lib/scope';

/** GET /api/fleet/machines?tag=a&tag=b&connectivity=&status=&search= */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'fleet.read');
  if (!isContext(ctx)) return ctx;
  const sp = new URL(request.url).searchParams;
  const ids = visibleMachineIds(ctx);
  // The whole fleet unless a page is asked for (the Machines screen shows all).
  const pr = readPage(request, { defaultLimit: Infinity });
  if (pr instanceof NextResponse) return pr;
  const { items: machines, page } = paged(listMachines({ orgIds: [ctx.orgId], ids, tags: sp.getAll('tag').filter(Boolean), connectivity: (sp.get('connectivity') as Connectivity) || undefined, status: (sp.get('status') as MachineStatus) || undefined, search: sp.get('search') || undefined }), pr);
  const backups = listBackups([ctx.orgId], ids);
  const open = listAlerts({ orgIds: [ctx.orgId], open: true, machineIds: ids });
  return pagedJson(request, {
    machines: machines.map((m) => {
      const mine = backups.filter((b) => b.machineId === m.id);
      const policies = policiesFor(m);
      return {
        ...m,
        backups: mine.length,
        failing: mine.filter((b) => b.lastStatus === 'error').length,
        backendSize: mine.reduce((a, b) => a + b.backendSize, 0),
        openAlerts: open.filter((a) => a.machineId === m.id).length,
        policyCompliance: { expected: policies.length, applied: policies.filter((p) => m.appliedPolicies[p.id] === p.version).length },
      };
    }),
    tagColors: tagColors(ctx.orgId),
  }, page);
}
