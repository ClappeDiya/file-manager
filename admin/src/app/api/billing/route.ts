import { NextRequest, NextResponse } from 'next/server';
import { isContext, requirePermission } from '@/lib/session';
import { license } from '@/lib/db/repos/orgs';
import { billingAccount, billingProvider, invoices, ownsLicense, trialEligible } from '@/lib/billing';
import { promotionCodesEnabled, trialDays } from '@/lib/billing/config';
import { PLANS, priceFor } from '@/lib/billing/plans';

/** GET /api/billing — license, subscription, plans and invoices of the current organization. */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'billing.read');
  if (!isContext(ctx)) return ctx;
  const provider = billingProvider();
  let list: Awaited<ReturnType<typeof invoices>> = [];
  let invoicesError: string | null = null;
  try {
    list = await invoices(ctx.orgId);
  } catch (e) {
    invoicesError = e instanceof Error ? e.message : String(e);
  }
  return NextResponse.json({
    configured: !!provider,
    provider: provider?.id ?? 'manual',
    ownsLicense: ownsLicense(ctx.orgId),
    license: license(ctx.orgId),
    account: billingAccount(ctx.orgId),
    plans: PLANS.map((p) => ({
      id: p.id,
      name: p.name,
      seats: p.seats,
      price: p.price,
      period: p.period,
      annualPrice: p.annualPrice ?? null,
      annualPeriod: p.annualPeriod ?? null,
      features: p.features,
      purchasable: !!provider && !!priceFor(p),
      annualPurchasable: !!provider && !!priceFor(p, 'year'),
    })),
    trial: { days: trialDays(), eligible: !!provider && ownsLicense(ctx.orgId) && trialEligible(ctx.orgId) },
    promotionCodes: promotionCodesEnabled(),
    invoices: list,
    invoicesError,
  });
}
