/** Billing: Stripe checkout/portal/invoices against a mock API, signed webhooks → license seats, idempotency, step-up. */
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import crypto from 'crypto';
import http from 'http';
import net from 'net';
import { NextRequest } from 'next/server';
import { db } from '@/lib/db';
import { listAudit } from '@/lib/db/repos/audit';
import { createOrg, license } from '@/lib/db/repos/orgs';
import { createSession } from '@/lib/db/repos/sessions';
import { billingAccount, setBillingProvider } from '@/lib/billing';
import { StripeProvider } from '@/lib/billing/stripe';
import { json, makeUser, req } from './helpers';
import * as billing from '@/app/api/billing/route';
import * as checkout from '@/app/api/billing/checkout/route';
import * as portal from '@/app/api/billing/portal/route';
import * as webhook from '@/app/api/billing/webhook/route';

const SECRET = 'whsec_test';
const PRICE = 'price_business_test';
let server: http.Server;
let base = '';
const calls: { method: string; url: string; body: string; auth: string }[] = [];

beforeAll(async () => {
  process.env.STRIPE_PRICE_BUSINESS = PRICE;
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
