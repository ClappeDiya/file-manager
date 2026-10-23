import { NextRequest, NextResponse } from 'next/server';
import { audit, isContext, readJson, requireStepUp } from '@/lib/session';
import { BillingError, startCheckout } from '@/lib/billing';
import { orgErrorResponse, publicOrigin } from '@/lib/route-helpers';

/** POST /api/billing/checkout {plan, seats} — a provider checkout URL for a new subscription. */
export async function POST(request: NextRequest) {
  const ctx = await requireStepUp(request, 'billing.write');
  if (!isContext(ctx)) return ctx;
  const b = (await readJson<{ plan?: string; seats?: number }>(request)) ?? {};
  try {
    const r = await startCheckout(ctx.orgId, b.plan ?? '', Number(b.seats), { email: ctx.user.email, origin: publicOrigin(request) });
    audit(ctx, { eventType: 'billing.checkout_start', description: `Started checkout for ${b.plan} with ${b.seats} seat(s)`, resourceType: 'billing' });
    return NextResponse.json(r);
  } catch (e) {
    if (e instanceof BillingError) return NextResponse.json({ error: e.message }, { status: e.status });
    return orgErrorResponse(e);
  }
}
