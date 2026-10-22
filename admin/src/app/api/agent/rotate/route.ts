import { NextRequest, NextResponse } from 'next/server';
import { agentAuth, isMachine } from '@/lib/agent-auth';
import { rotateToken } from '@/lib/db/repos/machines';
import { appendAudit } from '@/lib/db/repos/audit';

/** POST /api/agent/rotate — the agent replaces its token (the old one stops working). */
export async function POST(request: NextRequest) {
  const m = await agentAuth(request);
  if (!isMachine(m)) return m;
  const agentToken = rotateToken(m.id);
  appendAudit({ eventType: 'machine.token_rotate', orgId: m.orgId, description: `"${m.name}" rotated its agent token`, resourceType: 'machine', resourceId: m.id });
  return NextResponse.json({ agentToken });
}
