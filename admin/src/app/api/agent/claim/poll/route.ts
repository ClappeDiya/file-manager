import { NextRequest, NextResponse } from 'next/server';
import { readJson } from '@/lib/session';
import { collectClaim } from '@/lib/db/repos/machines';
import { OrgError } from '@/lib/db/repos/orgs';

/** POST /api/agent/claim/poll {claimId, secret} → `pending` until claimed, then the agent token once. */
export async function POST(request: NextRequest) {
  const b = await readJson<{ claimId?: string; secret?: string }>(request);
  if (!b?.claimId || !b.secret) return NextResponse.json({ error: 'claimId and secret are required' }, { status: 400 });
  try {
    const r = collectClaim(b.claimId, b.secret);
    return NextResponse.json(r.token ? { status: r.status, machineId: r.machineId, agentToken: r.token } : { status: r.status });
  } catch (e) {
    if (e instanceof OrgError) return NextResponse.json({ error: e.message }, { status: e.status });
    throw e;
  }
}
