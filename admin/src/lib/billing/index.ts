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
 * - A first subscription of an organization starts with a free trial
 *   (BILLING_TRIAL_DAYS, no card needed); an organization never gets a second
 *   one. While trialing, the subscribed plan and seats apply.
 * - Plans can be bought monthly or yearly; coupons are entered at checkout and
 *   the applied discount is kept for the billing page.
 */
import { db, now, parseJson, type Row } from '../db';
import { getOrg, updateOrg, OrgError } from '../db/repos/orgs';
import { appendAudit } from '../db/repos/audit';
import { listChannels } from '../db/repos/channels';
import { effectiveBranding } from '../branding';
import { deliverTo } from '../mailer';
import { promotionCodesEnabled, trialDays } from './config';
import { BILLING_INTERVALS, FREE_PLAN, planById, priceFor, priceMatch, type BillingInterval } from './plans';
import { BillingError, type BillingEvent, type BillingProvider, type Discount, type Invoice, type SubscriptionStatus } from './provider';
import { hasPaymentMethod, trialDaysLeft, trialEndsPhrase, trialOutcome } from './trial';
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
  /** Billing interval of the subscription (from its price). */
  interval: BillingInterval | null;
  /** End of the free trial, while the subscription is (or was) trialing. */
  trialEnd: string | null;
  /** The organization has had a subscription, so it gets no first-time trial. */
  trialUsed: boolean;
  /** The coupon applied to the subscription. */
  discount: Discount | null;
  /** The subscription has its own default payment method (null: not known yet). */
  subPaymentMethod: boolean | null;
  /** The customer has a default payment method (null: not known yet). */
  customerPaymentMethod: boolean | null;
  /** The unfinished checkout session, closed when a newer one is started. */
  checkoutSessionId: string | null;
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
    interval: r.billing_interval === 'month' || r.billing_interval === 'year' ? r.billing_interval : null,
    trialEnd: (r.trial_end as string) ?? null,
    trialUsed: !!r.trial_used,
    discount: parseJson<Discount | null>(r.discount, null),
    subPaymentMethod: r.sub_payment_method == null ? null : !!r.sub_payment_method,
    customerPaymentMethod: r.customer_payment_method == null ? null : !!r.customer_payment_method,
    checkoutSessionId: (r.checkout_session_id as string) ?? null,
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

const tri = (v: boolean | null) => (v === null ? null : v ? 1 : 0);

function upsert(orgId: string, provider: string, patch: Partial<Omit<BillingAccount, 'orgId' | 'provider' | 'updatedAt'>>): void {
  const cur = billingAccount(orgId);
  const next = { customerId: null, subscriptionId: null, plan: null, seats: null, status: 'none', currentPeriodEnd: null, cancelAtPeriodEnd: false, interval: null, trialEnd: null, trialUsed: false, discount: null, subPaymentMethod: null, customerPaymentMethod: null, checkoutSessionId: null, ...cur, ...patch };
  db()
    .prepare(
      `INSERT INTO billing_accounts (org_id, provider, customer_id, subscription_id, plan, seats, status, current_period_end, cancel_at_period_end, billing_interval, trial_end, trial_used, discount, sub_payment_method, customer_payment_method, checkout_session_id, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
       ON CONFLICT(org_id) DO UPDATE SET provider = excluded.provider, customer_id = excluded.customer_id, subscription_id = excluded.subscription_id, plan = excluded.plan, seats = excluded.seats, status = excluded.status, current_period_end = excluded.current_period_end, cancel_at_period_end = excluded.cancel_at_period_end, billing_interval = excluded.billing_interval, trial_end = excluded.trial_end, trial_used = excluded.trial_used, discount = excluded.discount, sub_payment_method = excluded.sub_payment_method, customer_payment_method = excluded.customer_payment_method, checkout_session_id = excluded.checkout_session_id, updated_at = excluded.updated_at`,
    )
    .run(orgId, provider, next.customerId, next.subscriptionId, next.plan, next.seats, next.status, next.currentPeriodEnd, next.cancelAtPeriodEnd ? 1 : 0, next.interval, next.trialEnd, next.trialUsed ? 1 : 0, next.discount ? JSON.stringify(next.discount) : null, tri(next.subPaymentMethod), tri(next.customerPaymentMethod), next.checkoutSessionId, now());
}

/** Organizations that own their license may buy; children inherit. */
export function ownsLicense(orgId: string): boolean {
  const o = getOrg(orgId);
  return !!o && (!o.parentId || o.detached);
}

const MAX_SEATS = 100_000;

/** Whether a new subscription of the organization starts with the free trial (first-time subscribers only). */
export function trialEligible(orgId: string): boolean {
  return trialDays() > 0 && !billingAccount(orgId)?.trialUsed;
}

export async function startCheckout(
  orgId: string,
  planId: string,
  seats: number,
  o: { email: string; origin: string; interval?: string },
): Promise<{ url: string }> {
  const p = billingProvider();
  if (!p) throw new OrgError('Online billing is not set up on this console', 409);
  if (!ownsLicense(orgId)) throw new OrgError('This organization’s license comes from its parent organization', 409);
  const interval = (o.interval ?? 'month') as BillingInterval;
  if (!BILLING_INTERVALS.includes(interval)) throw new OrgError('Choose monthly or annual billing');
  const plan = planById(planId);
  if (!plan || !priceFor(plan)) throw new OrgError('That plan cannot be bought online');
  const price = priceFor(plan, interval);
  if (!price) throw new OrgError(`${plan.name} is not available with annual billing`);
  if (!Number.isInteger(seats) || seats < plan.seats || seats > MAX_SEATS) throw new OrgError(`Choose ${plan.seats} to ${MAX_SEATS} seats`);
  const acct = billingAccount(orgId);
  if (acct && ['active', 'trialing', 'past_due'].includes(acct.status)) throw new OrgError('This organization already has a subscription; change it in the billing portal', 409);
  // The local flag is written by webhooks, so it lags: ask the provider whether this customer already has a
  // subscription before offering a trial (a second tab, or a trial the console never heard about).
  let trial = trialEligible(orgId) ? trialDays() : 0;
  if (trial > 0 && acct?.customerId && (await p.hasSubscriptions(acct.customerId))) {
    trial = 0;
    upsert(orgId, p.id, { trialUsed: true });
  }
  const session = await p.createCheckout({
    orgId,
    priceId: price,
    seats,
    customerId: acct?.customerId ?? null,
    email: o.email,
    successUrl: `${o.origin}/billing?checkout=success`,
    cancelUrl: `${o.origin}/billing?checkout=cancelled`,
    trialDays: trial,
    allowPromotionCodes: promotionCodesEnabled(),
  });
  // Only one checkout session stays open per organization: a session opened earlier (another tab, another
  // owner) was priced before this one and could still start a second, trialing subscription.
  // The stored id is swapped before anything is awaited: two starts that overlap each take over from the
  // other's id, so every earlier session gets expired and only the last one stays open.
  const earlier = billingAccount(orgId)?.checkoutSessionId;
  upsert(orgId, p.id, { checkoutSessionId: session.id });
  if (earlier && earlier !== session.id) await p.expireCheckout(earlier).catch(() => undefined); // already completed or expired
  return { url: session.url };
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

/** The organization an event belongs to: by metadata, else by billing customer. */
function resolveOrg(provider: string, orgId: string | null, customerId: string): string | null {
  return orgId && getOrg(orgId) ? orgId : (accountByCustomer(provider, customerId)?.orgId ?? null);
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
    const open = billingAccount(ev.orgId)?.checkoutSessionId;
    upsert(ev.orgId, provider, { customerId: ev.customerId, subscriptionId: ev.subscriptionId, ...(ev.subscriptionId ? { trialUsed: true } : {}), ...(open && open === ev.sessionId ? { checkoutSessionId: null } : {}) });
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
  if (ev.kind === 'customer') {
    const acct = accountByCustomer(provider, ev.customerId);
    record(acct?.orgId ?? null, 'customer', `payment method ${ev.hasPaymentMethod ? 'on file' : 'none'}`);
    if (!acct) return 'unmatched';
    upsert(acct.orgId, provider, { customerPaymentMethod: ev.hasPaymentMethod });
    return 'applied';
  }
  // Subscription: find the organization by metadata, else by customer.
  const orgId = resolveOrg(provider, ev.orgId, ev.customerId);
  if (ev.kind === 'trial_will_end') {
    record(orgId, 'trial_will_end', `subscription ${ev.subscriptionId}`);
    if (!orgId) return 'unmatched';
    upsert(orgId, provider, { subPaymentMethod: ev.hasPaymentMethod });
    const pm = hasPaymentMethod(billingAccount(orgId)!);
    const left = trialDaysLeft(ev.trialEnd);
    audit(orgId, 'billing.trial_will_end', `The free trial ends ${trialEndsPhrase(left)}${ev.trialEnd ? ` (${ev.trialEnd.slice(0, 10)})` : ''}; ${pm === true ? 'a payment method is on file, so it converts to a paid subscription' : pm === false ? 'add a payment method in the billing portal to keep the subscription' : 'check in the billing portal that a payment method is on file to keep the subscription'}`, pm === true ? 'info' : 'warning', { subscriptionId: ev.subscriptionId, trialEnd: ev.trialEnd, paymentMethod: pm });
    return 'applied';
  }
  if (!orgId) {
    record(null, 'subscription', `no organization for customer ${ev.customerId}`);
    return 'unmatched';
  }
  const match = ev.priceId ? priceMatch(ev.priceId) : null;
  const plan = match?.plan ?? null;
  // Any subscription (trial or not) uses up the organization's first-time trial.
  const sub = { customerId: ev.customerId, subscriptionId: ev.subscriptionId, status: ev.status, seats: ev.seats, currentPeriodEnd: ev.periodEnd, cancelAtPeriodEnd: ev.cancelAtPeriodEnd, interval: match?.interval ?? null, trialEnd: ev.trialEnd, trialUsed: true, discount: ev.discount, subPaymentMethod: ev.hasPaymentMethod };
  const keeps = !ev.deleted && KEEPS_LICENSE.includes(ev.status);
  if (keeps) {
    if (!plan) {
      record(orgId, 'subscription', `unknown price ${ev.priceId}`);
      audit(orgId, 'billing.subscription', `Subscription ${ev.subscriptionId} uses an unknown price (${ev.priceId}); license unchanged`, 'warning');
      upsert(orgId, provider, sub);
      return 'applied';
    }
    updateOrg(orgId, { licensePlan: plan.id, licenseSeats: ev.seats });
    upsert(orgId, provider, { ...sub, plan: plan.id });
    record(orgId, 'subscription', `${ev.status} ${plan.id} x${ev.seats}${match?.interval === 'year' ? ' (annual)' : ''}`);
    audit(
      orgId,
      'billing.subscription',
      `Subscription ${ev.status}: ${plan.name}, ${ev.seats} seat(s)${match?.interval === 'year' ? ', annual' : ''}${ev.status === 'trialing' && ev.trialEnd ? `, free trial until ${ev.trialEnd.slice(0, 10)}` : ''}${ev.cancelAtPeriodEnd ? ', ends at period end' : ''}${ev.status === 'past_due' ? ' — payment overdue' : ''}`,
      ev.status === 'past_due' ? 'warning' : 'info',
    );
    return 'applied';
  }
  updateOrg(orgId, { licensePlan: FREE_PLAN.id, licenseSeats: FREE_PLAN.seats });
  upsert(orgId, provider, { customerId: ev.customerId, subscriptionId: null, plan: FREE_PLAN.id, seats: FREE_PLAN.seats, status: ev.deleted ? 'canceled' : ev.status, currentPeriodEnd: null, cancelAtPeriodEnd: false, interval: null, trialEnd: null, trialUsed: true, discount: null, subPaymentMethod: null });
  record(orgId, 'subscription', `${ev.deleted ? 'deleted' : ev.status} -> ${FREE_PLAN.id}`);
  audit(orgId, 'billing.subscription', `Subscription ${ev.deleted ? 'ended' : ev.status}: back to ${FREE_PLAN.name} (${FREE_PLAN.seats} seats)`, 'warning');
  return 'applied';
}

/** Tell the organization's enabled channels (email, Slack, …) that the trial is about to end; failures are retried from the outbox. */
async function notifyTrialEnding(orgId: string, trialEnd: string | null): Promise<void> {
  const channelIds = listChannels(orgId)
    .filter((c) => c.enabled)
    .map((c) => c.id);
  if (!channelIds.length) return;
  const acct = billingAccount(orgId);
  const pm = acct ? hasPaymentMethod(acct) : null;
  const left = trialDaysLeft(trialEnd);
  const when = trialEndsPhrase(left);
  const org = getOrg(orgId);
  await deliverTo(
    orgId,
    channelIds,
    {
      subject: `[${effectiveBranding(orgId).displayName}] Your free trial ends ${when} — ${org?.name ?? orgId}`,
      text: `The free trial of ${org?.name ?? 'your organization'} ends ${when}${trialEnd ? ` (${trialEnd.slice(0, 10)})` : ''}. ${trialOutcome(pm)} Manage it on the Billing page (Manage subscription).`,
      data: { event: 'billing.trial_will_end', trialEnd },
    },
    'billing.trial_will_end',
  );
}

/** Verify and apply a webhook delivery. */
export async function handleBillingWebhook(rawBody: string, headers: Headers): Promise<ReturnType<typeof applyBillingEvent>> {
  const p = billingProvider();
  if (!p) throw new BillingError('Online billing is not set up on this console', 404);
  const ev = p.parseWebhook(rawBody, headers);
  const result = applyBillingEvent(p.id, ev);
  if (result === 'applied' && ev.kind === 'trial_will_end') {
    const orgId = resolveOrg(p.id, ev.orgId, ev.customerId);
    if (orgId) await notifyTrialEnding(orgId, ev.trialEnd);
  }
  return result;
}
