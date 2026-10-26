/**
 * Plans the console sells. Seats are enrolled machines (enforced at
 * registration). `priceEnv` names the environment variable holding the
 * provider's per-seat price id; a plan without one is not sold online.
 */
export type BillingInterval = 'month' | 'year';
export const BILLING_INTERVALS: BillingInterval[] = ['month', 'year'];

/** What the annual price saves against twelve monthly payments; the marketing site states the same figure. */
export const ANNUAL_DISCOUNT_PERCENT = 20;

export interface Plan {
  id: 'starter' | 'business' | 'enterprise';
  name: string;
  /** Seats included without a subscription (starter) or the minimum to buy. */
  seats: number;
  /** Per-seat price id at the billing provider (env var name). */
  priceEnv?: string;
  /** Per-seat price id of the annual price (env var name); without it the plan is monthly only. */
  annualPriceEnv?: string;
  /** Shown on the billing page. */
  price: string;
  period: string;
  /** Annual price per seat and its period label (shown with the annual toggle). */
  annualPrice?: string;
  annualPeriod?: string;
  features: string[];
}

export const PLANS: Plan[] = [
  { id: 'starter', name: 'Starter', seats: 5, price: '$0', period: 'Free', features: ['5 machines', 'Basic connectors (Local, SFTP)', '7-day audit log', 'Community support'] },
  {
    id: 'business',
    name: 'Business',
    seats: 1,
    priceEnv: 'STRIPE_PRICE_BUSINESS',
    annualPriceEnv: 'STRIPE_PRICE_BUSINESS_ANNUAL',
    price: '$9.99',
    period: '/machine/month',
    // 12 x $9.99 less ANNUAL_DISCOUNT_PERCENT, rounded to the cent. The amounts that are
    // actually charged are the Stripe prices; keep this in step with them.
    annualPrice: '$95.90',
    annualPeriod: '/machine/year',
    features: ['Any number of machines', 'All connectors', 'SAML SSO and SCIM', '90-day audit log', 'Policy engine', 'Approval workflows', 'Email support'],
  },
  {
    id: 'enterprise',
    name: 'Enterprise',
    seats: 0,
    price: 'Custom',
    period: 'Contact us',
    features: ['Everything in Business', 'SIEM integration', 'Unlimited audit log', 'Custom policies', 'Dedicated support', 'SLA guarantee'],
  },
];

/** The free tier an organization falls back to when its subscription ends. */
export const FREE_PLAN = PLANS[0];

export function planById(id: string): Plan | null {
  return PLANS.find((p) => p.id === id) ?? null;
}

/** The plan and billing interval sold under a provider price id (from the env mapping). */
export function priceMatch(priceId: string): { plan: Plan; interval: BillingInterval } | null {
  if (!priceId) return null;
  for (const plan of PLANS) {
    if (plan.priceEnv && process.env[plan.priceEnv] === priceId) return { plan, interval: 'month' };
    if (plan.annualPriceEnv && process.env[plan.annualPriceEnv] === priceId) return { plan, interval: 'year' };
  }
  return null;
}

/** The plan sold under a provider price id (from the env mapping). */
export function planForPrice(priceId: string): Plan | null {
  return priceMatch(priceId)?.plan ?? null;
}

/** The provider price id of a plan for an interval, or null when it is not sold that way. */
export function priceFor(plan: Plan, interval: BillingInterval = 'month'): string | null {
  const env = interval === 'year' ? plan.annualPriceEnv : plan.priceEnv;
  return env ? (process.env[env] || null) : null;
}
