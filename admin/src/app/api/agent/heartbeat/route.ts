import { NextRequest, NextResponse } from 'next/server';
import { clientIp, readJson } from '@/lib/session';
import { agentAuth, isMachine } from '@/lib/agent-auth';
import { touchMachine, type ClientInfo, type JobSummary } from '@/lib/db/repos/machines';
import { takeCommands } from '@/lib/db/repos/commands';
import { policySetVersion } from '@/lib/db/repos/fleet-policies';
import { syncBackupSchedules } from '@/lib/db/repos/reports';

/**
 * POST /api/agent/heartbeat {info, jobs, policySetVersion, heartbeatSeconds}
 * → queued commands and whether fleet policies changed.
 */
export async function POST(request: NextRequest) {
  const m = await agentAuth(request);
  if (!isMachine(m)) return m;
  const b = (await readJson<{ info?: ClientInfo; jobs?: JobSummary[]; policySetVersion?: string; heartbeatSeconds?: number }>(request)) ?? {};
  const jobs = Array.isArray(b.jobs) ? b.jobs.filter((j) => j && typeof j.id === 'string') : undefined;
  touchMachine(m.id, clientIp(request), { ...(b.info ?? {}), jobs, heartbeatSeconds: b.heartbeatSeconds });
  if (jobs) syncBackupSchedules(m.orgId, m.id, jobs.map((j) => ({ id: j.id, name: j.name, scheduleIntervalMinutes: j.scheduleIntervalMinutes ?? null })));
  const version = policySetVersion(m);
  return NextResponse.json({
    machineId: m.id,
    status: m.status,
    commands: takeCommands(m.id).map((c) => ({ id: c.id, kind: c.kind, payload: c.payload, expiresAt: c.expiresAt })),
    policySetVersion: version,
    policiesChanged: version !== (b.policySetVersion ?? ''),
    heartbeatSeconds: m.heartbeatSeconds,
  });
}
