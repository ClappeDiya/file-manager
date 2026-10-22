import { NextRequest, NextResponse } from 'next/server';
import { BillingError, handleBillingWebhook } from '@/lib/billing';
import { logger } from '@/lib/utils/logger';

/**
 * POST /api/billing/webhook — the billing provider's events. Authenticated by
 * the provider's signature over the raw body (no session); each event is
 * applied once.
 */
export async function POST(request: NextRequest) {
  const raw = await request.text();
  try {
    const result = handleBillingWebhook(raw, request.headers);
    return NextResponse.json({ received: true, result });
  } catch (e) {
    if (e instanceof BillingError) {
      logger.warn('billing.webhook_rejected', { error: e.message });
      return NextResponse.json({ error: e.message }, { status: e.status });
    }
    throw e;
  }
}
