import { NextRequest, NextResponse } from 'next/server';
import { paged } from '@/lib/pagination';
import { pagedJson, readPage } from '@/lib/route-helpers';
import { isContext, requirePermission } from '@/lib/session';
import { listMachines, type Machine } from '@/lib/db/repos/machines';
import { listBackups } from '@/lib/db/repos/reports';
import { policiesFor } from '@/lib/db/repos/fleet-policies';
import { listAlerts } from '@/lib/db/repos/alerts';
import { visibleMachineIds } from '@/lib/scope';
import type { DeviceHealth } from '@/lib/types/audit';

/**
 * GET /api/devices — device health derived from the registered machines
 * (connectivity, failing backups, fleet-policy compliance).
 */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'devices.read');
  if (!isContext(ctx)) return ctx;
  const { searchParams } = new URL(request.url);
  const status = searchParams.get('status');
  const compliant = searchParams.get('compliant');
  const ids = visibleMachineIds(ctx);
  const machines = listMachines({ orgIds: [ctx.orgId], ids });
  const backups = listBackups([ctx.orgId], ids);
  const alerts = listAlerts({ orgIds: [ctx.orgId], open: true, machineIds: ids });

  const toDevice = (m: Machine): DeviceHealth => {
    const failing = backups.filter((b) => b.machineId === m.id && b.lastStatus === 'error');
    const pending = policiesFor(m).filter((p) => m.appliedPolicies[p.id] !== p.version);
    const s: DeviceHealth['status'] = m.connectivity === 'online' ? (failing.length ? 'error' : alerts.some((a) => a.machineId === m.id) ? 'degraded' : 'online') : m.connectivity === 'stale' ? 'degraded' : 'offline';
    return {
      id: m.id,
      deviceId: m.id,
      deviceName: m.name,
      userId: '',
      userName: m.hostname,
      clientVersion: m.clientVersion,
      os: m.os,
      osVersion: m.osVersion,
      lastSeenAt: m.lastSeenAt ?? m.registeredAt,
      status: s,
      failurePatterns: failing.map((b) => ({ type: 'backup_failed', count: 1, lastOccurrence: b.lastRunAt ?? '', description: `Backup "${b.name}" failed` })),
      policyCompliance: {
        isCompliant: pending.length === 0,
        violations: pending.map((p) => ({ policyId: p.id, policyName: p.name, domain: 'backup', description: `Policy v${p.version} not applied yet`, severity: 'medium' as const, detectedAt: m.lastSeenAt ?? m.registeredAt })),
        lastCheckedAt: m.lastSeenAt ?? m.registeredAt,
      },
    };
  };

  let devices = machines.map(toDevice);
  if (status) devices = devices.filter((d) => d.status === status);
  if (compliant !== null) devices = devices.filter((d) => d.policyCompliance.isCompliant === (compliant === 'true'));
  const stats = {
    total: devices.length,
    online: devices.filter((d) => d.status === 'online').length,
    degraded: devices.filter((d) => d.status === 'degraded').length,
    offline: devices.filter((d) => d.status === 'offline').length,
    error: devices.filter((d) => d.status === 'error').length,
    nonCompliant: devices.filter((d) => !d.policyCompliance.isCompliant).length,
  };
  // Stats cover every matching device; the list is paged when asked.
  const pr = readPage(request, { defaultLimit: Infinity });
  if (pr instanceof NextResponse) return pr;
  const { items, page } = paged(devices, pr);
  return pagedJson(request, { devices: items, stats }, page);
}
