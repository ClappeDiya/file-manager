import { NextRequest, NextResponse } from 'next/server';
import { audit, isContext, requireStepUp } from '@/lib/session';
import { BillingError, openPortal } from '@/lib/billing';
import { orgErrorResponse, publicOrigin } from '@/lib/route-helpers';

/** POST /api/billing/portal — the provider's portal (seats, payment method, cancellation, invoices). */
export async function POST(request: NextRequest) {
  const ctx = await requireStepUp(request, 'billing.write');
  if (!isContext(ctx)) return ctx;
  try {
    const r = await openPortal(ctx.orgId, publicOrigin(request));
    audit(ctx, { eventType: 'billing.portal_open', description: 'Opened the billing portal', resourceType: 'billing' });
    return NextResponse.json(r);
  } catch (e) {
    if (e instanceof BillingError) return NextResponse.json({ error: e.message }, { status: e.status });
    return orgErrorResponse(e);
  }
}
