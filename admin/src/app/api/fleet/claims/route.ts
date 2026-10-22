import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { claimMachine } from '@/lib/db/repos/machines';
import { OrgError } from '@/lib/db/repos/orgs';
import { enrollLimiter } from '@/lib/rate-limit';

/** POST /api/fleet/claims {code, name?, tags?} — claim the machine showing this code into the current organization. */
export async function POST(request: NextRequest) {
  const ctx = await requirePermission(request, 'fleet.enroll');
  if (!isContext(ctx)) return ctx;
  if (enrollLimiter.wait(`claimcode:${ctx.user.id}`)) return badRequest('Too many wrong codes; wait a few minutes', 429);
  const b = await readJson<{ code?: string; name?: string; tags?: string[] }>(request);
  if (!b?.code) return badRequest('Enter the code shown on the machine');
  try {
    const m = claimMachine(ctx.orgId, b.code, { name: b.name, tags: b.tags }, ctx.user.email);
    audit(ctx, { eventType: 'machine.claim', description: `Claimed "${m.name}"`, resourceType: 'machine', resourceId: m.id });
    return NextResponse.json({ machine: m }, { status: 201 });
  } catch (e) {
    if (e instanceof OrgError) {
      enrollLimiter.fail(`claimcode:${ctx.user.id}`);
      return badRequest(e.message, e.status);
    }
    throw e;
  }
}
