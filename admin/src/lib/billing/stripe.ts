/**
 * Stripe over its REST API (no SDK): Checkout Sessions (subscription mode,
 * per-seat quantity), Billing Portal sessions, invoices, and webhook
 * verification (`Stripe-Signature: t=…,v1=HMAC-SHA256(secret, "t.payload")`,
 * 5-minute tolerance, constant-time compare).
 *
 * A trial is started with `subscription_data[trial_period_days]` and no card up
 * front (`payment_method_collection=if_required`); a trial that ends without a
 * payment method is cancelled by Stripe, which returns the organization to the
 * free plan through `customer.subscription.deleted`.
 *
 * STRIPE_SECRET_KEY, STRIPE_WEBHOOK_SECRET, STRIPE_API_BASE (tests/proxies).
 */
import crypto from 'crypto';
import { BillingError, type BillingEvent, type BillingProvider, type Discount, type Invoice, type SubscriptionStatus } from './provider';

const TOLERANCE_S = 300;

type Obj = Record<string, unknown>;
const str = (v: unknown) => (typeof v === 'string' ? v : null);
const iso = (v: unknown) => (v ? new Date(Number(v) * 1000).toISOString() : null);
const idOf = (v: unknown) => (typeof v === 'string' ? v : v && typeof v === 'object' ? (str((v as Obj).id) ?? '') : '');

/** Stripe's form encoding for nested parameters. */
function form(params: Record<string, string | number | undefined>): string {
  return Object.entries(params)
    .filter(([, v]) => v !== undefined && v !== '')
    .map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(String(v))}`)
    .join('&');
}

/** The coupon of a Stripe discount object, or null. */
function discountOf(v: unknown): Discount | null {
  if (!v || typeof v !== 'object') return null;
  const d = v as Obj;
  const c = (d.coupon ?? {}) as Obj;
  if (!c || typeof c !== 'object' || !Object.keys(c).length) return null;
  const dur = str(c.duration);
  return {
    name: str(c.name) ?? str(c.id) ?? 'Discount',
    percentOff: typeof c.percent_off === 'number' ? c.percent_off : null,
    amountOff: typeof c.amount_off === 'number' ? c.amount_off : null,
    currency: str(c.currency)?.toUpperCase() ?? null,
    duration: dur === 'once' || dur === 'repeating' || dur === 'forever' ? dur : null,
    endsAt: iso(d.end),
  };
}

export class StripeProvider implements BillingProvider {
  readonly id = 'stripe';
  constructor(
    private readonly key: string,
    private readonly webhookSecret: string,
    private readonly base = 'https://api.stripe.com',
  ) {}

  private async call(method: 'GET' | 'POST', path: string, params: Record<string, string | number | undefined> = {}): Promise<Obj> {
    const body = form(params);
    const url = `${this.base}${path}${method === 'GET' && body ? `?${body}` : ''}`;
    let res: Response;
    try {
      res = await fetch(url, {
        method,
        headers: { authorization: `Bearer ${this.key}`, ...(method === 'POST' ? { 'content-type': 'application/x-www-form-urlencoded' } : {}), 'stripe-version': '2024-06-20' },
        body: method === 'POST' ? body : undefined,
        signal: AbortSignal.timeout(20_000),
      });
    } catch (e) {
      throw new BillingError(`Stripe is unreachable: ${(e as Error).message}`);
    }
    const data = (await res.json().catch(() => ({}))) as Obj;
    if (!res.ok) throw new BillingError(`Stripe refused the request: ${str((data.error as Obj | undefined)?.message) ?? `HTTP ${res.status}`}`, res.status >= 500 ? 502 : 400);
    return data;
  }

  async createCheckout(o: Parameters<BillingProvider['createCheckout']>[0]) {
    const s = await this.call('POST', '/v1/checkout/sessions', {
      mode: 'subscription',
      'line_items[0][price]': o.priceId,
      'line_items[0][quantity]': o.seats,
      success_url: o.successUrl,
      cancel_url: o.cancelUrl,
      client_reference_id: o.orgId,
      customer: o.customerId ?? undefined,
      customer_email: o.customerId ? undefined : o.email,
      'subscription_data[metadata][org_id]': o.orgId,
      'metadata[org_id]': o.orgId,
      allow_promotion_codes: o.allowPromotionCodes ? 'true' : undefined,
      ...(o.trialDays && o.trialDays > 0
        ? {
            'subscription_data[trial_period_days]': o.trialDays,
            payment_method_collection: 'if_required',
            'subscription_data[trial_settings][end_behavior][missing_payment_method]': 'cancel',
          }
        : {}),
    });
    const url = str(s.url);
    if (!url) throw new BillingError('Stripe did not return a checkout URL');
    return { url };
  }

  async createPortal(o: { customerId: string; returnUrl: string }) {
    const s = await this.call('POST', '/v1/billing_portal/sessions', { customer: o.customerId, return_url: o.returnUrl });
    const url = str(s.url);
    if (!url) throw new BillingError('Stripe did not return a portal URL');
    return { url };
  }

  async listInvoices(customerId: string): Promise<Invoice[]> {
    const r = await this.call('GET', '/v1/invoices', { customer: customerId, limit: 24 });
    return ((r.data as Obj[] | undefined) ?? []).map((i) => ({
      id: idOf(i.id),
      number: str(i.number),
      status: str(i.status) ?? 'unknown',
      amountDue: Number(i.amount_due ?? 0),
      amountPaid: Number(i.amount_paid ?? 0),
      currency: (str(i.currency) ?? 'usd').toUpperCase(),
      createdAt: new Date(Number(i.created ?? 0) * 1000).toISOString(),
      url: str(i.hosted_invoice_url),
      pdf: str(i.invoice_pdf),
    }));
  }

  parseWebhook(rawBody: string, headers: Headers, now = new Date()): BillingEvent {
    const header = headers.get('stripe-signature') ?? '';
    const parts = header.split(',').map((p) => p.trim().split('=') as [string, string]);
    const t = Number(parts.find(([k]) => k === 't')?.[1]);
    const sigs = parts.filter(([k]) => k === 'v1').map(([, v]) => v);
    if (!Number.isFinite(t) || !sigs.length) throw new BillingError('Missing or malformed Stripe-Signature header', 400);
    if (Math.abs(now.getTime() / 1000 - t) > TOLERANCE_S) throw new BillingError('The webhook timestamp is too old or in the future', 400);
    const expected = crypto.createHmac('sha256', this.webhookSecret).update(`${t}.${rawBody}`).digest();
    const ok = sigs.some((s) => {
      const b = Buffer.from(s, 'hex');
      return b.length === expected.length && crypto.timingSafeEqual(b, expected);
    });
    if (!ok) throw new BillingError('The webhook signature does not match', 400);
    let ev: Obj;
    try {
      ev = JSON.parse(rawBody) as Obj;
    } catch {
      throw new BillingError('The webhook body is not JSON', 400);
    }
    const id = str(ev.id) ?? '';
    const type = str(ev.type) ?? '';
    const obj = ((ev.data as Obj | undefined)?.object ?? {}) as Obj;
    const meta = (obj.metadata ?? {}) as Obj;
    switch (type) {
      case 'checkout.session.completed':
        return { kind: 'checkout_completed', id, orgId: str(obj.client_reference_id) ?? str(meta.org_id), customerId: idOf(obj.customer), subscriptionId: idOf(obj.subscription) || null };
      case 'customer.subscription.created':
      case 'customer.subscription.updated':
      case 'customer.subscription.deleted': {
        const item = (((obj.items as Obj | undefined)?.data as Obj[] | undefined) ?? [])[0] ?? {};
        return {
          kind: 'subscription',
          id,
          orgId: str(meta.org_id),
          customerId: idOf(obj.customer),
          subscriptionId: idOf(obj.id),
          status: (str(obj.status) ?? 'canceled') as SubscriptionStatus,
          priceId: idOf(item.price) || null,
          seats: Number(item.quantity ?? obj.quantity ?? 0),
          periodEnd: iso(obj.current_period_end),
          cancelAtPeriodEnd: !!obj.cancel_at_period_end,
          deleted: type === 'customer.subscription.deleted',
          trialEnd: iso(obj.trial_end),
          discount: discountOf(obj.discount),
        };
      }
      case 'customer.subscription.trial_will_end':
        return { kind: 'trial_will_end', id, orgId: str(meta.org_id), customerId: idOf(obj.customer), subscriptionId: idOf(obj.id), trialEnd: iso(obj.trial_end) };
      case 'invoice.paid':
      case 'invoice.payment_failed':
        return {
          kind: 'invoice',
          id,
          customerId: idOf(obj.customer),
          paid: type === 'invoice.paid',
          amount: Number(obj.amount_due ?? 0),
          currency: (str(obj.currency) ?? 'usd').toUpperCase(),
          number: str(obj.number),
          url: str(obj.hosted_invoice_url),
        };
      default:
        return { kind: 'ignored', id, type };
    }
  }
}

/** The configured Stripe provider, or null when billing is not set up. */
export function stripeFromEnv(): StripeProvider | null {
  const key = process.env.STRIPE_SECRET_KEY;
  const secret = process.env.STRIPE_WEBHOOK_SECRET;
  if (!key || !secret) return null;
  return new StripeProvider(key, secret, process.env.STRIPE_API_BASE?.replace(/\/$/, '') || undefined);
}
