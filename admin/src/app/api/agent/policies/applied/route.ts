import { NextRequest, NextResponse } from 'next/server';
import { readJson } from '@/lib/session';
import { agentAuth, isMachine } from '@/lib/agent-auth';
import { setAppliedPolicies } from '@/lib/db/repos/machines';
import { appendAudit } from '@/lib/db/repos/audit';

/** POST /api/agent/policies/applied {applied: {policyId: version}, errors: {policyId: message}} */
export async function POST(request: NextRequest) {
  const m = await agentAuth(request);
  if (!isMachine(m)) return m;
  const b = await readJson<{ applied?: Record<string, number>; errors?: Record<string, string> }>(request);
  const applied = Object.fromEntries(Object.entries(b?.applied ?? {}).filter(([k, v]) => typeof k === 'string' && Number.isInteger(v)));
  setAppliedPolicies(m.id, applied);
  for (const [pid, msg] of Object.entries(b?.errors ?? {})) appendAudit({ eventType: 'policy.apply_failed', severity: 'warning', orgId: m.orgId, description: `"${m.name}" could not apply policy ${pid}: ${String(msg).slice(0, 300)}`, resourceType: 'machine', resourceId: m.id });
  return NextResponse.json({ ok: true });
}
