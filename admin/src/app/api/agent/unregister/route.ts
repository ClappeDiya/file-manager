import { NextRequest, NextResponse } from 'next/server';
import { agentAuth, isMachine } from '@/lib/agent-auth';
import { setMachineStatus } from '@/lib/db/repos/machines';
import { appendAudit } from '@/lib/db/repos/audit';

/** POST /api/agent/unregister — the machine leaves the console (its token is revoked). */
export async function POST(request: NextRequest) {
  const m = await agentAuth(request);
  if (!isMachine(m)) return m;
  setMachineStatus(m.id, 'revoked');
  appendAudit({ eventType: 'machine.unregister', orgId: m.orgId, description: `"${m.name}" unregistered itself`, resourceType: 'machine', resourceId: m.id });
  return NextResponse.json({ ok: true });
}
