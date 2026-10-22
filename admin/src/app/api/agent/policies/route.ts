import { NextRequest, NextResponse } from 'next/server';
import { agentAuth, isMachine } from '@/lib/agent-auth';
import { credentialsForPolicies, policiesFor, policySetVersion } from '@/lib/db/repos/fleet-policies';
import { appendAudit } from '@/lib/db/repos/audit';

/** GET /api/agent/policies — the job templates for this machine and the central credentials they need. */
export async function GET(request: NextRequest) {
  const m = await agentAuth(request);
  if (!isMachine(m)) return m;
  const policies = policiesFor(m);
  const credentials = credentialsForPolicies(m.orgId, policies);
  if (credentials.length) appendAudit({ eventType: 'credential.deliver', orgId: m.orgId, description: `Delivered ${credentials.length} credential(s) to "${m.name}"`, resourceType: 'machine', resourceId: m.id, metadata: { credentialIds: credentials.map((c) => c.id) } });
  return NextResponse.json({
    policySetVersion: policySetVersion(m),
    policies: policies.map((p) => ({ id: p.id, name: p.name, version: p.version, job: p.job })),
    credentials: credentials.map((c) => ({ credentialId: c.credentialId, name: c.name, secret: c.secret, updatedAt: c.updatedAt })),
  }, { headers: { 'cache-control': 'no-store' } });
}
