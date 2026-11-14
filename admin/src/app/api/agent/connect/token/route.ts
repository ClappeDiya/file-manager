import { NextRequest, NextResponse } from 'next/server';
import { clientIp, readJson } from '@/lib/session';
import { redeemConnectCode, type ClientInfo } from '@/lib/db/repos/machines';
import { getOrg, OrgError } from '@/lib/db/repos/orgs';
import { appendAudit } from '@/lib/db/repos/audit';
import { enrollLimiter } from '@/lib/rate-limit';

/** POST /api/agent/connect/token {code, codeVerifier, redirectUri, info, tags?} → machine id + agent token. */
export async function POST(request: NextRequest) {
  const ip = clientIp(request);
  if (enrollLimiter.wait(`connect:${ip}`)) return NextResponse.json({ error: 'Too many attempts' }, { status: 429 });
  const b = await readJson<{ code?: string; codeVerifier?: string; redirectUri?: string; info?: ClientInfo; tags?: string[] }>(request);
  if (!b?.code || !b.codeVerifier || !b.redirectUri) return NextResponse.json({ error: 'code, codeVerifier and redirectUri are required' }, { status: 400 });
  try {
    const { machine, token, by } = redeemConnectCode(b.code, b.codeVerifier, b.redirectUri, b.info ?? {}, b.tags ?? []);
    appendAudit({ eventType: 'machine.register', orgId: machine.orgId, userEmail: by, ipAddress: ip, description: `Machine "${machine.name}" registered by browser sign-in`, resourceType: 'machine', resourceId: machine.id, metadata: { via: machine.registeredVia } });
    return NextResponse.json({ machineId: machine.id, agentToken: token, organization: getOrg(machine.orgId)?.name ?? '', heartbeatSeconds: machine.heartbeatSeconds }, { status: 201 });
  } catch (e) {
    if (e instanceof OrgError) {
      enrollLimiter.fail(`connect:${ip}`);
      return NextResponse.json({ error: e.message }, { status: e.status });
    }
    throw e;
  }
}
