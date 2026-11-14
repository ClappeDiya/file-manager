import { NextRequest, NextResponse } from 'next/server';
import { clientIp, readJson } from '@/lib/session';
import { registerWithToken, type ClientInfo } from '@/lib/db/repos/machines';
import { getOrg, OrgError } from '@/lib/db/repos/orgs';
import { appendAudit } from '@/lib/db/repos/audit';
import { enrollLimiter } from '@/lib/rate-limit';

/** POST /api/agent/register {registrationToken, info, tags?} → machine id + agent token. */
export async function POST(request: NextRequest) {
  const ip = clientIp(request);
  if (enrollLimiter.wait(`reg:${ip}`)) return NextResponse.json({ error: 'Too many attempts' }, { status: 429 });
  const b = await readJson<{ registrationToken?: string; info?: ClientInfo; tags?: string[] }>(request);
  if (!b?.registrationToken) return NextResponse.json({ error: 'registrationToken is required' }, { status: 400 });
  try {
    const { machine, token } = registerWithToken(b.registrationToken, b.info ?? {}, b.tags ?? []);
    appendAudit({ eventType: 'machine.register', orgId: machine.orgId, ipAddress: ip, description: `Machine "${machine.name}" registered`, resourceType: 'machine', resourceId: machine.id, metadata: { via: machine.registeredVia } });
    return NextResponse.json({ machineId: machine.id, agentToken: token, organization: getOrg(machine.orgId)?.name ?? '', heartbeatSeconds: machine.heartbeatSeconds }, { status: 201 });
  } catch (e) {
    if (e instanceof OrgError) {
      enrollLimiter.fail(`reg:${ip}`);
      return NextResponse.json({ error: e.message }, { status: e.status });
    }
    throw e;
  }
}
