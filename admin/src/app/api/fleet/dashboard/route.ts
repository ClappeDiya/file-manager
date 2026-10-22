import { NextRequest, NextResponse } from 'next/server';
import { canIn, isContext, requirePermission } from '@/lib/session';
import { listMachines, type Connectivity } from '@/lib/db/repos/machines';
import { listBackups, listReports, storageHistory } from '@/lib/db/repos/reports';
import { listAlerts } from '@/lib/db/repos/alerts';
import { license, managedSubtree } from '@/lib/db/repos/orgs';
import { visibleMachineIds } from '@/lib/scope';

/**
 * GET /api/fleet/dashboard?tag=&days=30&includeChildren=1
 * Machine status (online/stale/offline), backup results and storage totals.
 */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'fleet.read');
  if (!isContext(ctx)) return ctx;
  const sp = new URL(request.url).searchParams;
  const days = Math.min(365, Math.max(1, Number(sp.get('days') ?? 30)));
  // Sub-organizations only where the user's own role reaches (a non-delegated group role does not).
  const orgIds = sp.get('includeChildren') === '1' && ctx.role !== 'user' ? managedSubtree(ctx.orgId).filter((id) => canIn(ctx, id, 'fleet.read')) : [ctx.orgId];
  const tags = sp.getAll('tag').filter(Boolean);
  const scoped = visibleMachineIds(ctx);
  const machines = listMachines({ orgIds, tags, ids: scoped });
  const ids = machines.map((m) => m.id);
  const backups = listBackups(orgIds, ids);
  const since = new Date(Date.now() - days * 86_400_000).toISOString();
  const reports = listReports({ orgIds, since, limit: 5000, machineIds: ids }).filter((r) => r.operation === 'backup');
  const count = (c: Connectivity) => machines.filter((m) => m.connectivity === c).length;
  const status = (s: string) => backups.filter((b) => b.lastStatus === s).length;
  const largest = [...backups].sort((a, b) => b.backendSize - a.backendSize).slice(0, 10);
  return NextResponse.json({
    machines: { total: machines.length, online: count('online'), stale: count('stale'), offline: count('offline'), never: count('never'), suspended: machines.filter((m) => m.status === 'suspended').length },
    backups: {
      total: backups.length,
      success: status('success'),
      warning: status('warning'),
      error: status('error'),
      neverRun: backups.filter((b) => !b.lastRunAt).length,
      sourceSize: backups.reduce((a, b) => a + b.sourceSize, 0),
      backendSize: backups.reduce((a, b) => a + b.backendSize, 0),
      versions: backups.reduce((a, b) => a + b.versionCount, 0),
    },
    runs: { total: reports.length, failed: reports.filter((r) => r.status === 'error').length, warnings: reports.filter((r) => r.status === 'warning').length, bytesUploaded: reports.reduce((a, r) => a + r.bytesUploaded, 0) },
    storageHistory: storageHistory(orgIds, days, ids),
    largestBackups: largest.map((b) => ({ ...b, machineName: machines.find((m) => m.id === b.machineId)?.name ?? '' })),
    openAlerts: listAlerts({ orgIds, open: true, machineIds: ids, limit: 20 }),
    license: license(ctx.orgId),
  });
}
