/**
 * Plans the console sells. Seats are enrolled machines (enforced at
 * registration). `priceEnv` names the environment variable holding the
 * provider's per-seat price id; a plan without one is not sold online.
 */
export interface Plan {
  id: 'starter' | 'business' | 'enterprise';
  name: string;
  /** Seats included without a subscription (starter) or the minimum to buy. */
  seats: number;
  /** Per-seat price id at the billing provider (env var name). */
  priceEnv?: string;
  /** Shown on the billing page. */
  price: string;
  period: string;
  features: string[];
}

export const PLANS: Plan[] = [
  { id: 'starter', name: 'Starter', seats: 5, price: '$0', period: 'Free', features: ['5 machines', 'Basic connectors (Local, SFTP)', '7-day audit log', 'Community support'] },
  {
    id: 'business',
    name: 'Business',
    seats: 1,
    priceEnv: 'STRIPE_PRICE_BUSINESS',
    price: '$9.99',
    period: '/machine/month',
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

/** The plan sold under a provider price id (from the env mapping). */
export function planForPrice(priceId: string): Plan | null {
  return PLANS.find((p) => p.priceEnv && process.env[p.priceEnv] === priceId) ?? null;
}

export function priceFor(plan: Plan): string | null {
  return plan.priceEnv ? (process.env[plan.priceEnv] ?? null) : null;
}
