/**
 * Billing: subscriptions bought through the provider become the
 * organization's license (plan + seats; seats are enrolled machines).
 *
 * - Only organizations that own their license (top-level or detached) buy;
 *   children inherit as before.
 * - Webhooks are verified, processed once (by event id) and drive the
 *   license: active/trialing/past_due (grace) keep the subscribed plan and
 *   seats; canceled/unpaid/expired subscriptions fall back to the free plan.
 * - Seat changes and payment details go through the provider's portal.
 */
import { db, now, type Row } from '../db';
import { getOrg, updateOrg, OrgError } from '../db/repos/orgs';
import { appendAudit } from '../db/repos/audit';
import { FREE_PLAN, planById, planForPrice, priceFor } from './plans';
import { BillingError, type BillingEvent, type BillingProvider, type Invoice, type SubscriptionStatus } from './provider';
import { stripeFromEnv } from './stripe';

export { BillingError };

let injected: BillingProvider | null | undefined;
/** Tests replace the provider. */
export function setBillingProvider(p: BillingProvider | null | undefined): void {
  injected = p;
}
export function billingProvider(): BillingProvider | null {
  return injected !== undefined ? injected : stripeFromEnv();
}

export interface BillingAccount {
  orgId: string;
  provider: string;
  customerId: string | null;
  subscriptionId: string | null;
  plan: string | null;
  seats: number | null;
  status: SubscriptionStatus | 'none';
  currentPeriodEnd: string | null;
  cancelAtPeriodEnd: boolean;
  updatedAt: string;
}

function fromRow(r: Row): BillingAccount {
  return {
    orgId: r.org_id as string,
    provider: r.provider as string,
    customerId: (r.customer_id as string) ?? null,
    subscriptionId: (r.subscription_id as string) ?? null,
    plan: (r.plan as string) ?? null,
    seats: r.seats === null ? null : Number(r.seats),
    status: r.status as BillingAccount['status'],
    currentPeriodEnd: (r.current_period_end as string) ?? null,
    cancelAtPeriodEnd: !!r.cancel_at_period_end,
    updatedAt: r.updated_at as string,
  };
}

export function billingAccount(orgId: string): BillingAccount | null {
  const r = db().prepare('SELECT * FROM billing_accounts WHERE org_id = ?').get(orgId) as Row | undefined;
  return r ? fromRow(r) : null;
}

function accountByCustomer(provider: string, customerId: string): BillingAccount | null {
  const r = db().prepare('SELECT * FROM billing_accounts WHERE provider = ? AND customer_id = ?').get(provider, customerId) as Row | undefined;
  return r ? fromRow(r) : null;
}

function upsert(orgId: string, provider: string, patch: Partial<Omit<BillingAccount, 'orgId' | 'provider' | 'updatedAt'>>): void {
  const cur = billingAccount(orgId);
  const next = { customerId: null, subscriptionId: null, plan: null, seats: null, status: 'none', currentPeriodEnd: null, cancelAtPeriodEnd: false, ...cur, ...patch };
  db()
    .prepare(
      `INSERT INTO billing_accounts (org_id, provider, customer_id, subscription_id, plan, seats, status, current_period_end, cancel_at_period_end, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
       ON CONFLICT(org_id) DO UPDATE SET provider = excluded.provider, customer_id = excluded.customer_id, subscription_id = excluded.subscription_id, plan = excluded.plan, seats = excluded.seats, status = excluded.status, current_period_end = excluded.current_period_end, cancel_at_period_end = excluded.cancel_at_period_end, updated_at = excluded.updated_at`,
    )
    .run(orgId, provider, next.customerId, next.subscriptionId, next.plan, next.seats, next.status, next.currentPeriodEnd, next.cancelAtPeriodEnd ? 1 : 0, now());
}

/** Organizations that own their license may buy; children inherit. */
export function ownsLicense(orgId: string): boolean {
  const o = getOrg(orgId);
  return !!o && (!o.parentId || o.detached);
}

const MAX_SEATS = 100_000;

export async function startCheckout(orgId: string, planId: string, seats: number, o: { email: string; origin: string }): Promise<{ url: string }> {
  const p = billingProvider();
  if (!p) throw new OrgError('Online billing is not set up on this console', 409);
  if (!ownsLicense(orgId)) throw new OrgError('This organization’s license comes from its parent organization', 409);
  const plan = planById(planId);
  const price = plan ? priceFor(plan) : null;
  if (!plan || !price) throw new OrgError('That plan cannot be bought online');
  if (!Number.isInteger(seats) || seats < plan.seats || seats > MAX_SEATS) throw new OrgError(`Choose ${plan.seats} to ${MAX_SEATS} seats`);
  const acct = billingAccount(orgId);
  if (acct && ['active', 'trialing', 'past_due'].includes(acct.status)) throw new OrgError('This organization already has a subscription; change it in the billing portal', 409);
  return p.createCheckout({ orgId, priceId: price, seats, customerId: acct?.customerId ?? null, email: o.email, successUrl: `${o.origin}/billing?checkout=success`, cancelUrl: `${o.origin}/billing?checkout=cancelled` });
}

export async function openPortal(orgId: string, origin: string): Promise<{ url: string }> {
  const p = billingProvider();
  const acct = billingAccount(orgId);
  if (!p) throw new OrgError('Online billing is not set up on this console', 409);
  if (!acct?.customerId) throw new OrgError('This organization has no billing account yet', 404);
  return p.createPortal({ customerId: acct.customerId, returnUrl: `${origin}/billing` });
}

export async function invoices(orgId: string): Promise<Invoice[]> {
  const p = billingProvider();
  const acct = billingAccount(orgId);
  return p && acct?.customerId ? p.listInvoices(acct.customerId) : [];
}

const KEEPS_LICENSE: SubscriptionStatus[] = ['active', 'trialing', 'past_due'];

function audit(orgId: string | null, eventType: string, description: string, severity: 'info' | 'warning' = 'info', metadata: Record<string, unknown> = {}) {
  appendAudit({ eventType, orgId: orgId ?? '', userEmail: 'billing', severity, description, resourceType: 'billing', metadata });
}

/** Apply one verified provider event (idempotent by event id). Returns what happened. */
export function applyBillingEvent(provider: string, ev: BillingEvent): 'duplicate' | 'ignored' | 'applied' | 'unmatched' {
  if (ev.id && db().prepare('SELECT 1 FROM billing_events WHERE id = ?').get(ev.id)) return 'duplicate';
  const record = (orgId: string | null, type: string, summary: string) =>
    db()
      .prepare('INSERT OR IGNORE INTO billing_events (id, provider, org_id, type, summary, received_at) VALUES (?, ?, ?, ?, ?, ?)')
      .run(ev.id || `anon_${Date.now()}`, provider, orgId, type, summary.slice(0, 300), now());
  if (ev.kind === 'ignored') {
    record(null, ev.type, 'ignored');
    return 'ignored';
  }
  if (ev.kind === 'checkout_completed') {
    if (!ev.orgId || !getOrg(ev.orgId)) {
      record(null, 'checkout_completed', `no organization for customer ${ev.customerId}`);
      return 'unmatched';
    }
    upsert(ev.orgId, provider, { customerId: ev.customerId, subscriptionId: ev.subscriptionId });
    record(ev.orgId, 'checkout_completed', `customer ${ev.customerId}`);
    audit(ev.orgId, 'billing.checkout', `Checkout completed (customer ${ev.customerId})`);
    return 'applied';
  }
  if (ev.kind === 'invoice') {
    const acct = accountByCustomer(provider, ev.customerId);
    record(acct?.orgId ?? null, ev.paid ? 'invoice_paid' : 'invoice_failed', `${ev.number ?? ''} ${ev.amount} ${ev.currency}`);
    audit(
      acct?.orgId ?? null,
      ev.paid ? 'billing.invoice_paid' : 'billing.payment_failed',
      `${ev.paid ? 'Invoice paid' : 'Payment failed'}: ${ev.number ?? 'invoice'} ${(ev.amount / 100).toFixed(2)} ${ev.currency}`,
      ev.paid ? 'info' : 'warning',
      { url: ev.url },
    );
    return acct ? 'applied' : 'unmatched';
  }
  // Subscription: find the organization by metadata, else by customer.
  const orgId = ev.orgId && getOrg(ev.orgId) ? ev.orgId : (accountByCustomer(provider, ev.customerId)?.orgId ?? null);
  if (!orgId) {
    record(null, 'subscription', `no organization for customer ${ev.customerId}`);
    return 'unmatched';
  }
  const plan = ev.priceId ? planForPrice(ev.priceId) : null;
  const keeps = !ev.deleted && KEEPS_LICENSE.includes(ev.status);
  if (keeps) {
    if (!plan) {
      record(orgId, 'subscription', `unknown price ${ev.priceId}`);
      audit(orgId, 'billing.subscription', `Subscription ${ev.subscriptionId} uses an unknown price (${ev.priceId}); license unchanged`, 'warning');
      upsert(orgId, provider, { customerId: ev.customerId, subscriptionId: ev.subscriptionId, status: ev.status, seats: ev.seats, currentPeriodEnd: ev.periodEnd, cancelAtPeriodEnd: ev.cancelAtPeriodEnd });
      return 'applied';
    }
    updateOrg(orgId, { licensePlan: plan.id, licenseSeats: ev.seats });
    upsert(orgId, provider, { customerId: ev.customerId, subscriptionId: ev.subscriptionId, plan: plan.id, seats: ev.seats, status: ev.status, currentPeriodEnd: ev.periodEnd, cancelAtPeriodEnd: ev.cancelAtPeriodEnd });
    record(orgId, 'subscription', `${ev.status} ${plan.id} x${ev.seats}`);
    audit(
      orgId,
      'billing.subscription',
      `Subscription ${ev.status}: ${plan.name}, ${ev.seats} seat(s)${ev.cancelAtPeriodEnd ? ', ends at period end' : ''}${ev.status === 'past_due' ? ' — payment overdue' : ''}`,
      ev.status === 'past_due' ? 'warning' : 'info',
    );
    return 'applied';
  }
  updateOrg(orgId, { licensePlan: FREE_PLAN.id, licenseSeats: FREE_PLAN.seats });
  upsert(orgId, provider, { customerId: ev.customerId, subscriptionId: null, plan: FREE_PLAN.id, seats: FREE_PLAN.seats, status: ev.deleted ? 'canceled' : ev.status, currentPeriodEnd: null, cancelAtPeriodEnd: false });
  record(orgId, 'subscription', `${ev.deleted ? 'deleted' : ev.status} -> ${FREE_PLAN.id}`);
  audit(orgId, 'billing.subscription', `Subscription ${ev.deleted ? 'ended' : ev.status}: back to ${FREE_PLAN.name} (${FREE_PLAN.seats} seats)`, 'warning');
  return 'applied';
}

/** Verify and apply a webhook delivery. */
export function handleBillingWebhook(rawBody: string, headers: Headers): ReturnType<typeof applyBillingEvent> {
  const p = billingProvider();
  if (!p) throw new BillingError('Online billing is not set up on this console', 404);
  return applyBillingEvent(p.id, p.parseWebhook(rawBody, headers));
}
