/**
 * Billing: Stripe checkout/portal/invoices against a mock API, signed webhooks → license seats, idempotency, step-up;
 * free trial (first subscription only), annual interval, trial-ending notices and applied coupons.
 */
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import crypto from 'crypto';
import http from 'http';
import net from 'net';
import { NextRequest } from 'next/server';
import Database from 'better-sqlite3';
import { db, migrate } from '@/lib/db';
import { listAudit } from '@/lib/db/repos/audit';
import { createOrg, license } from '@/lib/db/repos/orgs';
import { createChannel } from '@/lib/db/repos/channels';
import { createLink, registerWithToken } from '@/lib/db/repos/machines';
import { listOutbox } from '@/lib/db/repos/outbox';
import { createSession } from '@/lib/db/repos/sessions';
import { billingAccount, setBillingProvider, trialEligible } from '@/lib/billing';
import { trialDaysLeft, discountAmount, discountDuration } from '@/lib/billing/trial';
import { DEFAULT_TRIAL_DAYS } from '@/lib/billing/config';
import { StripeProvider } from '@/lib/billing/stripe';
import { json, makeUser, req } from './helpers';
import * as billing from '@/app/api/billing/route';
import * as checkout from '@/app/api/billing/checkout/route';
import * as portal from '@/app/api/billing/portal/route';
import * as webhook from '@/app/api/billing/webhook/route';

const SECRET = 'whsec_test';
const PRICE = 'price_business_test';
const ANNUAL = 'price_business_annual_test';
let server: http.Server;
let base = '';
const calls: { method: string; url: string; body: string; auth: string }[] = [];

beforeAll(async () => {
  process.env.STRIPE_PRICE_BUSINESS = PRICE;
  process.env.STRIPE_PRICE_BUSINESS_ANNUAL = ANNUAL;
  server = http.createServer((rq, rs) => {
    let body = '';
    rq.on('data', (c) => (body += c));
    rq.on('end', () => {
      calls.push({ method: rq.method ?? '', url: rq.url ?? '', body, auth: String(rq.headers.authorization) });
      const send = (status: number, v: unknown) => rs.writeHead(status, { 'content-type': 'application/json' }).end(JSON.stringify(v));
      if (rq.url === '/v1/checkout/sessions') return send(200, { id: 'cs_1', url: 'https://checkout.stripe.test/cs_1' });
      if (rq.url === '/v1/billing_portal/sessions') return send(200, { id: 'bps_1', url: 'https://billing.stripe.test/p/1' });
      if (rq.url?.startsWith('/v1/invoices')) {
        if (rq.url.includes('cus_broken')) return send(500, { error: { message: 'boom' } });
        return send(200, {
          data: [{ id: 'in_1', number: 'INV-1', status: 'paid', amount_due: 2997, amount_paid: 2997, currency: 'usd', created: 1_700_000_000, hosted_invoice_url: 'https://inv/1', invoice_pdf: 'https://inv/1.pdf' }],
        });
      }
      send(404, { error: { message: 'no such route' } });
    });
  });
  await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
  base = `http://127.0.0.1:${(server.address() as net.AddressInfo).port}`;
  setBillingProvider(new StripeProvider('sk_test', SECRET, base));
});

afterAll(() => {
  setBillingProvider(undefined);
  delete process.env.STRIPE_PRICE_BUSINESS;
  delete process.env.STRIPE_PRICE_BUSINESS_ANNUAL;
  server.close();
});

const sign = (body: string, t = Math.floor(Date.now() / 1000), secret = SECRET) => `t=${t},v1=${crypto.createHmac('sha256', secret).update(`${t}.${body}`).digest('hex')}`;

async function deliver(event: unknown, signature?: string) {
  const body = JSON.stringify(event);
  const r = new NextRequest(new URL('http://localhost:3001/api/billing/webhook'), { method: 'POST', headers: { 'content-type': 'application/json', 'stripe-signature': signature ?? sign(body) }, body });
  return json<{ result?: string; error?: string }>(await webhook.POST(r));
}

let n = 0;
const subscription = (orgId: string, o: { status?: string; seats?: number; price?: string; type?: string; customer?: string; cancel?: boolean } = {}) => ({
  id: `evt_${++n}`,
  type: o.type ?? 'customer.subscription.updated',
  data: {
    object: {
      id: 'sub_1',
      customer: o.customer ?? `cus_${orgId}`,
      status: o.status ?? 'active',
      metadata: { org_id: orgId },
      current_period_end: 1_900_000_000,
      cancel_at_period_end: !!o.cancel,
      items: { data: [{ price: { id: o.price ?? PRICE }, quantity: o.seats ?? 25 }] },
    },
  },
});

describe('billing', () => {
  it('creates a checkout session with per-seat quantity and the organization as reference', async () => {
    const org = createOrg({ name: 'Buyer' });
    const owner = makeUser('owner@buyer.test', 'org_owner', org.id);
    const r = await json<{ url: string }>(await checkout.POST(await req('POST', '/api/billing/checkout', { as: owner, body: { plan: 'business', seats: 12 } })));
    expect(r).toEqual({ status: 200, body: { url: 'https://checkout.stripe.test/cs_1' } });
    const c = calls.at(-1)!;
    expect(c.auth).toBe('Bearer sk_test');
    const p = new URLSearchParams(c.body);
    expect(p.get('mode')).toBe('subscription');
    expect(p.get('line_items[0][price]')).toBe(PRICE);
    expect(p.get('line_items[0][quantity]')).toBe('12');
    expect(p.get('client_reference_id')).toBe(org.id);
    expect(p.get('subscription_data[metadata][org_id]')).toBe(org.id);
    expect(p.get('customer_email')).toBe('owner@buyer.test');
    expect(p.get('success_url')).toMatch(/\/billing\?checkout=success$/);
    expect(listAudit({ eventType: 'billing.checkout_start' }).entries[0].orgId).toBe(org.id);
  });

  it('refuses unsellable plans, bad seat counts, children and admins without billing.write', async () => {
    const org = createOrg({ name: 'Picky' });
    const owner = makeUser('owner@picky.test', 'org_owner', org.id);
    const post = async (body: unknown, as = owner) => (await checkout.POST(await req('POST', '/api/billing/checkout', { as, body }))).status;
    expect(await post({ plan: 'enterprise', seats: 10 })).toBe(400);
    expect(await post({ plan: 'business', seats: 0 })).toBe(400);
    expect(await post({ plan: 'business', seats: 2.5 })).toBe(400);
    expect(await post({ plan: 'business', seats: 5 }, makeUser('admin@picky.test', 'org_admin', org.id))).toBe(403);
    const child = createOrg({ name: 'Picky child', parentId: org.id });
    expect(await post({ plan: 'business', seats: 5 }, makeUser('owner@child.test', 'org_owner', child.id))).toBe(409);
  });

  it('requires a recent sign-in (step-up) to start checkout or open the portal', async () => {
    const org = createOrg({ name: 'Stale' });
    const owner = makeUser('owner@stale.test', 'org_owner', org.id);
    const s = createSession(owner.id, { hours: 8 });
    const old = new Date(Date.now() - 3600_000).toISOString();
    db().prepare('UPDATE sessions SET created_at = ?, verified_at = ? WHERE id = ?').run(old, old, s.id);
    for (const [h, path] of [
      [checkout.POST, '/api/billing/checkout'],
      [portal.POST, '/api/billing/portal'],
    ] as const) {
      const r = await json<{ code?: string }>(await h(await req('POST', path, { as: owner, sid: s.id, body: { plan: 'business', seats: 5 } })));
      expect(r.body.code).toBe('reauth_required');
    }
  });

  it('applies a signed subscription to the license, lists invoices and opens the portal', async () => {
    const org = createOrg({ name: 'Subscribed' });
    const owner = makeUser('owner@sub.test', 'org_owner', org.id);
    expect((await deliver({ id: 'evt_cs', type: 'checkout.session.completed', data: { object: { client_reference_id: org.id, customer: `cus_${org.id}`, subscription: 'sub_1' } } })).body.result).toBe('applied');
    expect((await deliver(subscription(org.id, { type: 'customer.subscription.created', seats: 25 }))).body.result).toBe('applied');
    expect(license(org.id)).toMatchObject({ plan: 'business', seats: 25 });
    expect(billingAccount(org.id)).toMatchObject({ customerId: `cus_${org.id}`, subscriptionId: 'sub_1', plan: 'business', seats: 25, status: 'active', currentPeriodEnd: new Date(1_900_000_000_000).toISOString() });

    const g = await json<{ configured: boolean; ownsLicense: boolean; plans: { id: string; purchasable: boolean }[]; invoices: { number: string; amountDue: number; currency: string }[] }>(
      await billing.GET(await req('GET', '/api/billing', { as: owner })),
    );
    expect(g.body.configured).toBe(true);
    expect(g.body.ownsLicense).toBe(true);
    expect(g.body.plans.map((p) => [p.id, p.purchasable])).toEqual([
      ['starter', false],
      ['business', true],
      ['enterprise', false],
    ]);
    expect(g.body.invoices).toEqual([expect.objectContaining({ number: 'INV-1', amountDue: 2997, currency: 'USD' })]);
    expect(calls.at(-1)!.url).toContain(`customer=cus_${org.id}`);

    const p = await json<{ url: string }>(await portal.POST(await req('POST', '/api/billing/portal', { as: owner })));
    expect(p.body.url).toBe('https://billing.stripe.test/p/1');
    expect(new URLSearchParams(calls.at(-1)!.body).get('customer')).toBe(`cus_${org.id}`);

    // An active subscription goes through the portal, not a second checkout.
    expect((await checkout.POST(await req('POST', '/api/billing/checkout', { as: owner, body: { plan: 'business', seats: 5 } }))).status).toBe(409);

    // Seat change, then cancellation → back to the free plan.
    await deliver(subscription(org.id, { seats: 40 }));
    expect(license(org.id).seats).toBe(40);
    await deliver(subscription(org.id, { type: 'customer.subscription.deleted', status: 'canceled' }));
    expect(license(org.id)).toMatchObject({ plan: 'starter', seats: 5 });
    expect(billingAccount(org.id)).toMatchObject({ status: 'canceled', subscriptionId: null });
    expect(listAudit({ eventType: 'billing.subscription' }).entries.find((e) => e.orgId === org.id)!.description).toContain('back to Starter');
  });

  it('keeps the license through past_due (grace) and drops it when unpaid', async () => {
    const org = createOrg({ name: 'Late payer' });
    await deliver(subscription(org.id, { seats: 10 }));
    await deliver(subscription(org.id, { seats: 10, status: 'past_due' }));
    expect(license(org.id)).toMatchObject({ plan: 'business', seats: 10 });
    expect(billingAccount(org.id)!.status).toBe('past_due');
    await deliver(subscription(org.id, { seats: 10, status: 'unpaid' }));
    expect(license(org.id)).toMatchObject({ plan: 'starter', seats: 5 });
  });

  it('processes each event once', async () => {
    const org = createOrg({ name: 'Dupes' });
    const ev = subscription(org.id, { seats: 7 });
    expect((await deliver(ev)).body.result).toBe('applied');
    await deliver(subscription(org.id, { seats: 9 }));
    expect((await deliver(ev)).body.result).toBe('duplicate');
    expect(license(org.id).seats).toBe(9);
  });

  it('leaves the license unchanged for an unknown price', async () => {
    const org = createOrg({ name: 'Odd price' });
    await deliver(subscription(org.id, { seats: 8 }));
    await deliver(subscription(org.id, { seats: 99, price: 'price_other' }));
    expect(license(org.id)).toMatchObject({ plan: 'business', seats: 8 });
    expect(listAudit({ eventType: 'billing.subscription' }).entries.find((e) => e.orgId === org.id)!.description).toContain('unknown price');
  });

  it('rejects unsigned, wrongly signed, stale and malformed deliveries without applying them', async () => {
    const org = createOrg({ name: 'Forged' });
    const ev = subscription(org.id, { seats: 500 });
    const body = JSON.stringify(ev);
    expect((await deliver(ev, '')).status).toBe(400);
    expect((await deliver(ev, sign(body, undefined, 'whsec_wrong'))).status).toBe(400);
    expect((await deliver(ev, sign(body, Math.floor(Date.now() / 1000) - 600))).body.error).toContain('too old');
    expect((await deliver(ev, sign(body).replace(/v1=../, 'v1=00'))).status).toBe(400);
    expect(license(org.id).seats).not.toBe(500);
    expect(db().prepare('SELECT 1 FROM billing_events WHERE id = ?').get(ev.id)).toBeUndefined();
    // A valid signature still works afterwards.
    expect((await deliver(ev)).body.result).toBe('applied');
  });

  it('matches by customer when metadata is missing, records unmatched and ignored events', async () => {
    const org = createOrg({ name: 'By customer' });
    await deliver({ id: 'evt_cs2', type: 'checkout.session.completed', data: { object: { client_reference_id: org.id, customer: 'cus_bycust', subscription: 'sub_x' } } });
    const ev = subscription(org.id, { customer: 'cus_bycust', seats: 3 });
    (ev.data.object.metadata as Record<string, string>) = {};
    expect((await deliver(ev)).body.result).toBe('applied');
    expect(license(org.id).seats).toBe(3);
    const lost = subscription('org_missing', { customer: 'cus_nobody' });
    expect((await deliver(lost)).body.result).toBe('unmatched');
    expect((await deliver({ id: 'evt_other', type: 'customer.created', data: { object: {} } })).body.result).toBe('ignored');
    expect((await deliver({ id: 'evt_inv', type: 'invoice.payment_failed', data: { object: { customer: 'cus_bycust', amount_due: 900, currency: 'usd', number: 'INV-9' } } })).body.result).toBe('applied');
    expect(listAudit({ eventType: 'billing.payment_failed' }).entries[0]).toMatchObject({ orgId: org.id, severity: 'warning' });
  });

  it('reports invoice errors without failing the page, and is unconfigured without a provider', async () => {
    const org = createOrg({ name: 'Broken invoices' });
    const owner = makeUser('owner@broken.test', 'org_owner', org.id);
    await deliver({ id: 'evt_cs3', type: 'checkout.session.completed', data: { object: { client_reference_id: org.id, customer: 'cus_broken', subscription: 'sub_b' } } });
    const g = await json<{ invoices: unknown[]; invoicesError: string | null }>(await billing.GET(await req('GET', '/api/billing', { as: owner })));
    expect(g.status).toBe(200);
    expect(g.body.invoices).toEqual([]);
    expect(g.body.invoicesError).toContain('boom');

    setBillingProvider(null);
    try {
      const u = await json<{ configured: boolean; provider: string; plans: { purchasable: boolean }[] }>(await billing.GET(await req('GET', '/api/billing', { as: owner })));
      expect(u.body).toMatchObject({ configured: false, provider: 'manual' });
      expect(u.body.plans.every((p) => !p.purchasable)).toBe(true);
      expect((await deliver(subscription(org.id))).status).toBe(404);
      expect((await checkout.POST(await req('POST', '/api/billing/checkout', { as: owner, body: { plan: 'business', seats: 5 } }))).status).toBe(409);
    } finally {
      setBillingProvider(new StripeProvider('sk_test', SECRET, base));
    }
  });
});

// ---------------------------------------------------------------------------------------------
// Trial, annual billing, trial-ending notices and coupons
// ---------------------------------------------------------------------------------------------

const DAY = 86_400_000;
const unix = (ms: number) => Math.floor(ms / 1000);

/** A subscription event carrying trial and discount data. */
function trialSub(orgId: string, o: { status?: string; price?: string; seats?: number; trialEnd?: number | null; type?: string; discount?: unknown; customer?: string; sub?: string } = {}) {
  const ev = subscription(orgId, { status: o.status ?? 'trialing', price: o.price, seats: o.seats ?? 5, type: o.type, customer: o.customer });
  const obj = ev.data.object as Record<string, unknown>;
  obj.id = o.sub ?? 'sub_t';
  obj.trial_end = o.trialEnd === undefined ? unix(Date.now() + 10 * DAY) : o.trialEnd;
  obj.discount = o.discount ?? null;
  return ev;
}

async function startCheckoutFor(orgName: string, body: Record<string, unknown> = { plan: 'business', seats: 3 }) {
  const org = createOrg({ name: orgName });
  const owner = makeUser(`owner@${orgName.toLowerCase().replace(/\W+/g, '')}.test`, 'org_owner', org.id);
  const post = async (b: Record<string, unknown> = body) => json<{ url?: string; error?: string }>(await checkout.POST(await req('POST', '/api/billing/checkout', { as: owner, body: b })));
  return { org, owner, post, params: () => new URLSearchParams(calls.at(-1)!.body) };
}

describe('billing: free trial', () => {
  it('starts a first-time subscription with the promised trial, no card up front, and a promotion-code field', async () => {
    const t = await startCheckoutFor('First timer');
    expect((await t.post()).status).toBe(200);
    const p = t.params();
    expect(p.get('line_items[0][price]')).toBe(PRICE);
    expect(p.get('subscription_data[trial_period_days]')).toBe(String(DEFAULT_TRIAL_DAYS));
    expect(DEFAULT_TRIAL_DAYS).toBe(14);
    expect(p.get('payment_method_collection')).toBe('if_required');
    expect(p.get('subscription_data[trial_settings][end_behavior][missing_payment_method]')).toBe('cancel');
    expect(p.get('allow_promotion_codes')).toBe('true');
  });

  it('takes the trial length from BILLING_TRIAL_DAYS; 0 turns trials off and a bad value falls back to the default', async () => {
    const t = await startCheckoutFor('Configured trial');
    try {
      process.env.BILLING_TRIAL_DAYS = '30';
      await t.post();
      expect(t.params().get('subscription_data[trial_period_days]')).toBe('30');
      process.env.BILLING_TRIAL_DAYS = '0';
      await t.post();
      expect([...t.params().keys()].filter((k) => k.includes('trial') || k === 'payment_method_collection')).toEqual([]);
      process.env.BILLING_TRIAL_DAYS = 'soon';
      await t.post();
      expect(t.params().get('subscription_data[trial_period_days]')).toBe('14');
    } finally {
      delete process.env.BILLING_TRIAL_DAYS;
    }
  });

  it('hides the promotion-code field when BILLING_ALLOW_PROMOTION_CODES is off', async () => {
    const t = await startCheckoutFor('No coupons');
    try {
      process.env.BILLING_ALLOW_PROMOTION_CODES = 'false';
      await t.post();
      expect(t.params().has('allow_promotion_codes')).toBe(false);
    } finally {
      delete process.env.BILLING_ALLOW_PROMOTION_CODES;
    }
  });

  it('gives an organization one trial only: none after any earlier subscription, even a cancelled one', async () => {
    const t = await startCheckoutFor('Repeat trialer');
    expect(trialEligible(t.org.id)).toBe(true);
    // Abandoning a checkout (no provider event) keeps the trial available.
    await t.post();
    await t.post();
    expect(t.params().get('subscription_data[trial_period_days]')).toBe('14');

    await deliver(trialSub(t.org.id, { customer: 'cus_repeat' }));
    expect(billingAccount(t.org.id)).toMatchObject({ status: 'trialing', trialUsed: true });
    expect(trialEligible(t.org.id)).toBe(false);
    await deliver(trialSub(t.org.id, { customer: 'cus_repeat', type: 'customer.subscription.deleted', status: 'canceled' }));
    expect(billingAccount(t.org.id)).toMatchObject({ status: 'canceled', trialUsed: true, trialEnd: null });

    expect((await t.post()).status).toBe(200);
    const p = t.params();
    expect(p.get('line_items[0][price]')).toBe(PRICE);
    expect([...p.keys()].filter((k) => k.includes('trial') || k === 'payment_method_collection')).toEqual([]);
    expect(p.get('customer')).toBe('cus_repeat');
  });

  it('counts a completed checkout and a paid subscription as the first-time trial used', async () => {
    const a = await startCheckoutFor('Checkout done');
    await deliver({ id: 'evt_cd', type: 'checkout.session.completed', data: { object: { client_reference_id: a.org.id, customer: 'cus_cd', subscription: 'sub_cd' } } });
    expect(trialEligible(a.org.id)).toBe(false);
    const b = await startCheckoutFor('Paid directly');
    await deliver(subscription(b.org.id, { status: 'active' }));
    expect(trialEligible(b.org.id)).toBe(false);
  });

  it('reports trial eligibility and annual availability on the billing page', async () => {
    const t = await startCheckoutFor('Page data');
    const get = async () => json<{ trial: { days: number; eligible: boolean }; promotionCodes: boolean; plans: { id: string; annualPrice: string | null; annualPeriod: string | null; annualPurchasable: boolean }[]; account: { trialEnd: string | null; interval: string | null } | null }>(await billing.GET(await req('GET', '/api/billing', { as: t.owner })));
    let g = await get();
    expect(g.body.trial).toEqual({ days: 14, eligible: true });
    expect(g.body.promotionCodes).toBe(true);
    expect(g.body.plans.map((p) => [p.id, p.annualPurchasable])).toEqual([
      ['starter', false],
      ['business', true],
      ['enterprise', false],
    ]);
    expect(g.body.plans[1]).toMatchObject({ annualPrice: '$95.90', annualPeriod: '/machine/year' });
    const end = unix(Date.now() + 9 * DAY);
    await deliver(trialSub(t.org.id, { trialEnd: end, price: ANNUAL }));
    g = await get();
    expect(g.body.trial.eligible).toBe(false);
    expect(g.body.account).toMatchObject({ trialEnd: new Date(end * 1000).toISOString(), interval: 'year' });
  });
});

describe('billing: trialing subscriptions', () => {
  it('grants the subscribed plan and seats during the trial and enforces the seat limit', async () => {
    const org = createOrg({ name: 'Trial seats' });
    await deliver(trialSub(org.id, { seats: 2 }));
    expect(license(org.id)).toMatchObject({ plan: 'business', seats: 2 });
    expect(billingAccount(org.id)).toMatchObject({ status: 'trialing', plan: 'business', seats: 2, interval: 'month' });
    const { token } = createLink(org.id, {}, 'tester');
    for (let i = 0; i < 2; i++) registerWithToken(token, { name: `m${i}` });
    expect(() => registerWithToken(token, { name: 'm3' })).toThrow(/license allows 2 machine/);
    // Converting to a paid subscription keeps the plan; the trial end is cleared when it is no longer trialing.
    await deliver(trialSub(org.id, { seats: 4, status: 'active', trialEnd: unix(Date.now() - DAY) }));
    expect(license(org.id).seats).toBe(4);
    expect(billingAccount(org.id)!.status).toBe('active');
  });

  it('drops back to the free plan when a trial ends without a payment method (subscription deleted)', async () => {
    const org = createOrg({ name: 'Trial lapses' });
    await deliver(trialSub(org.id, { seats: 6, sub: 'sub_lapse' }));
    await deliver(trialSub(org.id, { type: 'customer.subscription.deleted', status: 'canceled', sub: 'sub_lapse' }));
    expect(license(org.id)).toMatchObject({ plan: 'starter', seats: 5 });
    expect(billingAccount(org.id)).toMatchObject({ status: 'canceled', trialUsed: true });
  });

  it('records the interval of the subscribed price', async () => {
    const m = createOrg({ name: 'Monthly' });
    const y = createOrg({ name: 'Yearly' });
    await deliver(trialSub(m.id, { status: 'active' }));
    await deliver(trialSub(y.id, { status: 'active', price: ANNUAL, seats: 12 }));
    expect(billingAccount(m.id)!.interval).toBe('month');
    expect(billingAccount(y.id)).toMatchObject({ interval: 'year', plan: 'business', seats: 12 });
    expect(license(y.id)).toMatchObject({ plan: 'business', seats: 12 });
  });
});

describe('billing: annual interval', () => {
  it('checks out the annual price when asked and rejects unknown or unavailable intervals', async () => {
    const t = await startCheckoutFor('Annual buyer');
    expect((await t.post({ plan: 'business', seats: 4, interval: 'year' })).status).toBe(200);
    expect(t.params().get('line_items[0][price]')).toBe(ANNUAL);
    expect(t.params().get('line_items[0][quantity]')).toBe('4');
    expect((await t.post({ plan: 'business', seats: 4, interval: 'month' })).status).toBe(200);
    expect(t.params().get('line_items[0][price]')).toBe(PRICE);
    expect((await t.post({ plan: 'business', seats: 4, interval: 'weekly' })).status).toBe(400);
    delete process.env.STRIPE_PRICE_BUSINESS_ANNUAL;
    try {
      const r = await t.post({ plan: 'business', seats: 4, interval: 'year' });
      expect(r.status).toBe(400);
      expect(r.body.error).toContain('annual');
    } finally {
      process.env.STRIPE_PRICE_BUSINESS_ANNUAL = ANNUAL;
    }
  });
});

describe('billing: trial ending notice', () => {
  let hookServer: http.Server;
  let hookUrl = '';
  const hookBodies: string[] = [];
  beforeAll(async () => {
    hookServer = http.createServer((rq, rs) => {
      let b = '';
      rq.on('data', (c) => (b += c));
      rq.on('end', () => {
        hookBodies.push(b);
        rs.statusCode = 200;
        rs.end();
      });
    });
    await new Promise<void>((r) => hookServer.listen(0, '127.0.0.1', r));
    hookUrl = `http://127.0.0.1:${(hookServer.address() as net.AddressInfo).port}/hook`;
  });
  afterAll(() => hookServer.close());

  const trialWillEnd = (orgId: string, trialEnd: number | null, id = `evt_twe_${++n}`) => ({
    id,
    type: 'customer.subscription.trial_will_end',
    data: { object: { id: 'sub_t', customer: `cus_${orgId}`, metadata: { org_id: orgId }, trial_end: trialEnd } },
  });

  it('audits the notice, delivers it to the organization’s channels once and shows it nowhere else', async () => {
    const org = createOrg({ name: 'Ending trial' });
    const ch = createChannel(org.id, { name: 'ops', kind: 'webhook', config: { url: hookUrl } });
    await deliver(trialSub(org.id));
    const ev = trialWillEnd(org.id, unix(Date.now() + 2 * DAY + 3600_000));
    hookBodies.length = 0;
    expect((await deliver(ev)).body.result).toBe('applied');
    const audited = listAudit({ eventType: 'billing.trial_will_end' }).entries.find((e) => e.orgId === org.id)!;
    expect(audited).toMatchObject({ severity: 'warning', resourceType: 'billing' });
    expect(audited.description).toContain('in 3 day(s)');
    expect(hookBodies).toHaveLength(1);
    expect(JSON.parse(hookBodies[0])).toMatchObject({ event: 'billing.trial_will_end', subject: expect.stringContaining('Your free trial ends in 3 days') });
    expect(listOutbox(org.id, { status: 'sent' }).map((e) => e.channelId)).toEqual([ch.id]);
    // A redelivered event is not notified again.
    expect((await deliver(ev)).body.result).toBe('duplicate');
    expect(hookBodies).toHaveLength(1);
  });

  it('still audits when the organization has no channel, and ignores events for unknown organizations', async () => {
    const org = createOrg({ name: 'No channels' });
    await deliver(trialSub(org.id));
    expect((await deliver(trialWillEnd(org.id, unix(Date.now() + DAY)))).body.result).toBe('applied');
    expect(listAudit({ eventType: 'billing.trial_will_end' }).entries.some((e) => e.orgId === org.id)).toBe(true);
    expect(listOutbox(org.id, {})).toHaveLength(0);
    const lost = trialWillEnd('org_missing', null);
    (lost.data.object as Record<string, unknown>).customer = 'cus_nobody';
    expect((await deliver(lost)).body.result).toBe('unmatched');
  });
});

describe('billing: coupons', () => {
  const coupon = (c: Record<string, unknown>, end: number | null = null) => ({ coupon: { id: 'cp_1', ...c }, end });

  it('keeps the applied discount for the billing page and clears it when the coupon is removed or the subscription ends', async () => {
    const org = createOrg({ name: 'Discounted' });
    const owner = makeUser('owner@discounted.test', 'org_owner', org.id);
    const end = unix(Date.now() + 60 * DAY);
    await deliver(trialSub(org.id, { status: 'active', discount: coupon({ name: 'Launch 20', percent_off: 20, duration: 'repeating' }, end) }));
    expect(billingAccount(org.id)!.discount).toEqual({ name: 'Launch 20', percentOff: 20, amountOff: null, currency: null, duration: 'repeating', endsAt: new Date(end * 1000).toISOString() });
    const g = await json<{ account: { discount: { name: string } } }>(await billing.GET(await req('GET', '/api/billing', { as: owner })));
    expect(g.body.account.discount.name).toBe('Launch 20');

    await deliver(trialSub(org.id, { status: 'active', discount: coupon({ amount_off: 500, currency: 'usd', duration: 'once' }) }));
    expect(billingAccount(org.id)!.discount).toEqual({ name: 'cp_1', percentOff: null, amountOff: 500, currency: 'USD', duration: 'once', endsAt: null });

    await deliver(trialSub(org.id, { status: 'active', discount: null }));
    expect(billingAccount(org.id)!.discount).toBeNull();

    await deliver(trialSub(org.id, { status: 'active', discount: coupon({ percent_off: 10, duration: 'forever' }) }));
    expect(billingAccount(org.id)!.discount).toMatchObject({ percentOff: 10, duration: 'forever' });
    await deliver(trialSub(org.id, { type: 'customer.subscription.deleted', status: 'canceled', discount: coupon({ percent_off: 10, duration: 'forever' }) }));
    expect(billingAccount(org.id)!.discount).toBeNull();
  });
});

describe('billing: schema upgrade', () => {
  it('marks organizations that already subscribed as having used their trial', () => {
    const d = new Database(':memory:');
    migrate(d);
    // Roll the billing table back to its version-33 shape, as a console upgraded from before trials would have it.
    for (const c of ['billing_interval', 'trial_end', 'trial_used', 'discount']) d.exec(`ALTER TABLE billing_accounts DROP COLUMN ${c}`);
    d.pragma('user_version = 33');
    const org = (id: string) => d.prepare("INSERT INTO organizations (id, name, created_at, updated_at) VALUES (?, ?, 'now', 'now')").run(id, id);
    for (const id of ['org_old_sub', 'org_old_customer_only']) org(id);
    d.prepare("INSERT INTO billing_accounts (org_id, provider, customer_id, subscription_id, plan, seats, status, updated_at) VALUES ('org_old_sub', 'stripe', 'cus_1', 'sub_1', 'business', 5, 'active', 'now')").run();
    d.prepare("INSERT INTO billing_accounts (org_id, provider, customer_id, status, updated_at) VALUES ('org_old_customer_only', 'stripe', 'cus_2', 'none', 'now')").run();
    expect(migrate(d)).toBe(1);
    expect(d.prepare('SELECT org_id, trial_used FROM billing_accounts ORDER BY org_id').all()).toEqual([
      { org_id: 'org_old_customer_only', trial_used: 0 },
      { org_id: 'org_old_sub', trial_used: 1 },
    ]);
  });
});

describe('billing: marketing copy', () => {
  it('quotes the trial length and prices the code actually uses', async () => {
    // The pricing page and cards import these constants; they must match what checkout does.
    const copy = await import('../../../marketing/src/lib/pricing');
    const { PLANS, ANNUAL_DISCOUNT_PERCENT } = await import('@/lib/billing/plans');
    const business = PLANS.find((p) => p.id === 'business')!;
    expect(copy.TRIAL_DAYS).toBe(DEFAULT_TRIAL_DAYS);
    expect(copy.BUSINESS_MONTHLY_PRICE).toBe(business.price);
    expect(copy.BUSINESS_ANNUAL_PRICE).toBe(business.annualPrice);
    expect(copy.ANNUAL_SAVINGS_PERCENT).toBe(ANNUAL_DISCOUNT_PERCENT);
    const monthly = Number(business.price.slice(1));
    const yearly = Number(business.annualPrice!.slice(1));
    expect(yearly).toBeCloseTo(monthly * 12 * (1 - ANNUAL_DISCOUNT_PERCENT / 100), 2);
  });
});

describe('billing: display helpers', () => {
  it('counts whole days left in a trial', () => {
    const at = new Date('2026-01-01T00:00:00Z');
    expect(trialDaysLeft('2026-01-04T00:00:00Z', at)).toBe(3);
    expect(trialDaysLeft('2026-01-01T06:00:00Z', at)).toBe(1);
    expect(trialDaysLeft('2025-12-30T00:00:00Z', at)).toBe(0);
    expect(trialDaysLeft(null, at)).toBeNull();
    expect(trialDaysLeft('not a date', at)).toBeNull();
  });

  it('describes discounts', () => {
    const base = { name: 'x', percentOff: null, amountOff: null, currency: null, duration: null, endsAt: null };
    expect(discountAmount({ ...base, percentOff: 20 })).toBe('20% off');
    expect(discountAmount({ ...base, percentOff: 12.5 })).toBe('12.5% off');
    expect(discountAmount({ ...base, amountOff: 500, currency: 'USD' })).toContain('5.00');
    expect(discountDuration({ ...base, duration: 'forever' })).toBe('every invoice');
    expect(discountDuration({ ...base, duration: 'once' })).toBe('the first invoice');
  });
});
