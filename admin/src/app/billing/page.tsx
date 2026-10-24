'use client';

/** Billing: the organization's license, its subscription (checkout and portal at the provider) and invoices. */
import React, { useState } from 'react';
import { useSearchParams } from 'next/navigation';
import { CreditCard, Users, CalendarClock, Check } from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { StatCard } from '@/components/ui/stat-card';
import { ErrorBanner, Loading } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { useAuthStore } from '@/lib/stores/auth-store';
import { formatDate } from '@/lib/utils/format';
import type { BillingAccount } from '@/lib/billing';
import { ANNUAL_DISCOUNT_PERCENT, type BillingInterval } from '@/lib/billing/plans';
import type { Invoice } from '@/lib/billing/provider';
import { discountAmount, discountDuration, hasPaymentMethod, trialDaysLeft, trialOutcome } from '@/lib/billing/trial';
import type { License } from '@/lib/db/repos/orgs';

interface PlanView {
  id: string;
  name: string;
  seats: number;
  price: string;
  period: string;
  annualPrice: string | null;
  annualPeriod: string | null;
  features: string[];
  purchasable: boolean;
  annualPurchasable: boolean;
}

interface Data {
  configured: boolean;
  provider: string;
  ownsLicense: boolean;
  license: License;
  account: BillingAccount | null;
  plans: PlanView[];
  trial: { days: number; eligible: boolean };
  promotionCodes: boolean;
  invoices: Invoice[];
  invoicesError: string | null;
}

const money = (cents: number, currency: string) => new Intl.NumberFormat(undefined, { style: 'currency', currency }).format(cents / 100);

export default function BillingPage() {
  const params = useSearchParams();
  const { data, error, loading } = useApi<Data>('/api/billing');
  const canWrite = useAuthStore((s) => s.checkPermission)('billing.write');
  const [seats, setSeats] = useState<Record<string, string>>({});
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [interval, setInterval] = useState<BillingInterval>('month');
  if (loading && !data) return <Loading />;
  if (!data) return <ErrorBanner message={error ?? 'Could not load billing'} />;

  const lic = data.license;
  const acct = data.account;
  const subscribed = !!acct && ['active', 'trialing', 'past_due'].includes(acct.status);
  const trialing = acct?.status === 'trialing';
  const paymentMethod = acct ? hasPaymentMethod(acct) : null;
  const daysLeft = trialing ? trialDaysLeft(acct.trialEnd ?? acct.currentPeriodEnd) : null;
  const annualOffered = data.plans.some((p) => p.annualPurchasable);
  const go = async (path: string, body?: unknown) => {
    setBusy(true);
    setErr(null);
    try {
      const r = await api<{ url: string }>(path, { body: body ?? {} });
      window.location.assign(r.url);
    } catch (e) {
      setErr(errorText(e));
      setBusy(false);
    }
  };
  const planName = (id: string | null) => data.plans.find((p) => p.id === id)?.name ?? (id ? id[0].toUpperCase() + id.slice(1) : '—');

  return (
    <div>
      <PageHeader title="Billing" description="Your license, subscription and invoices" />
      <ErrorBanner message={err} />
      {params.get('checkout') === 'success' && (
        <div className="mb-4 rounded-md border border-success bg-success-bg px-3 py-2 text-sm">Thank you — your subscription is being confirmed and the license updates in a moment.</div>
      )}
      {trialing && (
        <div className="mb-4 rounded-md border border-info bg-info-bg px-3 py-2 text-sm" role="status">
          Free trial: {daysLeft === null ? 'ends soon' : daysLeft <= 0 ? 'ends today' : `${daysLeft} day${daysLeft === 1 ? '' : 's'} left`}
          {acct.trialEnd || acct.currentPeriodEnd ? ` (until ${formatDate((acct.trialEnd ?? acct.currentPeriodEnd) as string)})` : ''}. {trialOutcome(paymentMethod)}
        </div>
      )}
      {acct?.status === 'past_due' && (
        <div className="mb-4 rounded-md border border-warning bg-warning-bg px-3 py-2 text-sm">A payment is overdue. The license stays active for now; update the payment method in the billing portal.</div>
      )}
      {!data.configured && <p className="mb-4 text-sm text-foreground-secondary">Online billing is not set up on this console; licenses are assigned by a parent organization or the operator.</p>}
      {data.configured && !data.ownsLicense && <p className="mb-4 text-sm text-foreground-secondary">This organization’s license comes from its parent organization.</p>}

      <div className="mb-6 grid grid-cols-1 gap-4 sm:grid-cols-3">
        <StatCard
          title="Current plan"
          value={planName(lic.plan)}
          icon={CreditCard}
          variant="default"
          subtitle={lic.inherited ? 'Inherited from the parent organization' : acct ? `Subscription: ${acct.status}` : 'Set on this organization'}
        />
        <StatCard title="Seats" value={lic.seats != null ? `${lic.used} / ${lic.seats}` : `${lic.used} used`} icon={Users} variant="default" subtitle="Enrolled machines" />
        <StatCard
          title={trialing ? 'Trial ends' : 'Renews'}
          value={acct?.currentPeriodEnd ? formatDate(acct.currentPeriodEnd) : '—'}
          icon={CalendarClock}
          variant="default"
          subtitle={acct?.cancelAtPeriodEnd ? 'Ends at the end of the period' : trialing ? (paymentMethod ? 'Converts to a paid subscription' : 'Converts only with a payment method') : subscribed ? `Renews automatically${acct?.interval === 'year' ? ' (annual)' : acct?.interval === 'month' ? ' (monthly)' : ''}` : 'No subscription'}
        />
      </div>

      {acct?.discount && (
        <div className="mb-4 rounded-md border border-success bg-success-bg px-3 py-2 text-sm" role="status">
          Discount applied: <strong>{acct.discount.name}</strong> — {discountAmount(acct.discount)}
          {discountDuration(acct.discount) ? `, ${discountDuration(acct.discount)}` : ''}.
        </div>
      )}

      {subscribed && canWrite && (
        <Button className="mb-6" disabled={busy} onClick={() => void go('/api/billing/portal')}>
          Manage subscription (seats, payment, invoices)
        </Button>
      )}

      <div className="mb-4 flex items-center justify-between gap-4">
        <h2 className="text-lg font-semibold text-foreground">Plans</h2>
        {annualOffered && (
          <div className="inline-flex rounded-md border border-border p-0.5 text-sm" role="group" aria-label="Billing interval">
            {(['month', 'year'] as const).map((i) => (
              <button
                key={i}
                type="button"
                aria-pressed={interval === i}
                onClick={() => setInterval(i)}
                className={`rounded px-3 py-1 ${interval === i ? 'bg-primary text-primary-foreground' : 'text-foreground-secondary hover:text-foreground'}`}
              >
                {i === 'month' ? 'Monthly' : `Annual (save ${ANNUAL_DISCOUNT_PERCENT}%)`}
              </button>
            ))}
          </div>
        )}
      </div>
      {data.configured && data.trial.eligible && data.plans.some((p) => p.purchasable) && (
        <p className="mb-4 text-sm text-foreground-secondary">Your first subscription starts with a {data.trial.days}-day free trial; no card is needed to start.</p>
      )}
      <div className="mb-6 grid grid-cols-1 gap-4 md:grid-cols-3">
        {data.plans.map((plan) => {
          const current = lic.plan === plan.id;
          const yearly = interval === 'year' && plan.annualPurchasable;
          const cost = yearly ? (plan.annualPrice ?? plan.price) : plan.price;
          const per = yearly ? (plan.annualPeriod ?? plan.period) : plan.period;
          const buyable = interval === 'year' ? plan.annualPurchasable : plan.purchasable;
          return (
            <Card key={plan.id} className={current ? 'border-primary ring-1 ring-primary' : ''}>
              <div className="mb-4 flex items-center justify-between">
                <h3 className="text-lg font-bold text-foreground">{plan.name}</h3>
                {current && <Badge variant="info">Current plan</Badge>}
              </div>
              <div className="mb-4">
                <span className="text-3xl font-bold text-foreground">{cost}</span>
                <span className="text-sm text-foreground-secondary"> {per}</span>
              </div>
              <ul className="mb-6 space-y-2">
                {plan.features.map((f) => (
                  <li key={f} className="flex items-center gap-2 text-sm text-foreground-secondary">
                    <Check size={14} className="shrink-0 text-success" />
                    {f}
                  </li>
                ))}
              </ul>
              {plan.id === 'enterprise' ? (
                <Button variant="primary" className="w-full" onClick={() => window.open('https://filemanager.clappe.com/pricing', '_blank', 'noopener')}>
                  Contact sales
                </Button>
              ) : buyable && canWrite && data.ownsLicense && !subscribed ? (
                <div className="flex items-end gap-2">
                  <Input
                    label="Seats"
                    type="number"
                    min={plan.seats}
                    className="w-24"
                    value={seats[plan.id] ?? String(Math.max(plan.seats, lic.used, 1))}
                    onChange={(e) => setSeats({ ...seats, [plan.id]: e.target.value })}
                  />
                  <Button className="flex-1" disabled={busy} onClick={() => void go('/api/billing/checkout', { plan: plan.id, seats: Number(seats[plan.id] ?? Math.max(plan.seats, lic.used, 1)), interval })}>
                    {data.trial.eligible && data.trial.days > 0 ? `Start ${data.trial.days}-day free trial` : 'Subscribe'}
                  </Button>
                </div>
              ) : (
                <Button variant="outline" className="w-full" disabled>
                  {current ? 'Current plan' : subscribed ? 'Change in the portal' : 'Not available online'}
                </Button>
              )}
            </Card>
          );
        })}
      </div>

      <Card>
        <CardHeader>
          <CardTitle>Invoices</CardTitle>
        </CardHeader>
        <CardContent>
          <ErrorBanner message={data.invoicesError} />
          {!data.invoices.length ? (
            <div className="py-8 text-center text-sm text-foreground-tertiary">No invoices yet.</div>
          ) : (
            <ul className="divide-y divide-border text-sm">
              {data.invoices.map((i) => (
                <li key={i.id} className="flex items-center justify-between py-2">
                  <span>
                    {i.number ?? i.id} · {formatDate(i.createdAt)} · {money(i.amountDue, i.currency)} <Badge variant={i.status === 'paid' ? 'success' : i.status === 'open' ? 'warning' : 'default'}>{i.status}</Badge>
                  </span>
                  <span className="flex gap-2">
                    {i.url && (
                      <a className="text-primary underline" href={i.url} target="_blank" rel="noopener noreferrer">
                        View
                      </a>
                    )}
                    {i.pdf && (
                      <a className="text-primary underline" href={i.pdf} target="_blank" rel="noopener noreferrer">
                        PDF
                      </a>
                    )}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
