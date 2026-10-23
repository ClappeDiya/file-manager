/**
 * Billing provider abstraction: where checkout and the self-service portal
 * live, how webhooks are verified and read, and where invoices come from.
 * Stripe is the implementation shipped (`stripe.ts`); without one the
 * console runs "manual" billing (licenses set by a parent organization).
 */

/** A provider event reduced to what the console acts on. */
export type BillingEvent =
  | { kind: 'checkout_completed'; id: string; orgId: string | null; customerId: string; subscriptionId: string | null }
  | {
      kind: 'subscription';
      id: string;
      orgId: string | null;
      customerId: string;
      subscriptionId: string;
      status: SubscriptionStatus;
      priceId: string | null;
      seats: number;
      periodEnd: string | null;
      cancelAtPeriodEnd: boolean;
      deleted: boolean;
    }
  | { kind: 'invoice'; id: string; customerId: string; paid: boolean; amount: number; currency: string; number: string | null; url: string | null }
  | { kind: 'ignored'; id: string; type: string };

export type SubscriptionStatus = 'trialing' | 'active' | 'past_due' | 'unpaid' | 'canceled' | 'incomplete' | 'incomplete_expired' | 'paused';

export interface Invoice {
  id: string;
  number: string | null;
  status: string;
  amountDue: number;
  amountPaid: number;
  currency: string;
  createdAt: string;
  url: string | null;
  pdf: string | null;
}

export class BillingError extends Error {
  constructor(
    message: string,
    readonly status = 502,
  ) {
    super(message);
  }
}

export interface BillingProvider {
  readonly id: string;
  createCheckout(o: { orgId: string; priceId: string; seats: number; customerId: string | null; email: string; successUrl: string; cancelUrl: string }): Promise<{ url: string }>;
  createPortal(o: { customerId: string; returnUrl: string }): Promise<{ url: string }>;
  /** Verify the signature and read the event; throws BillingError(400) on a bad signature. */
  parseWebhook(rawBody: string, headers: Headers, now?: Date): BillingEvent;
  listInvoices(customerId: string): Promise<Invoice[]>;
}
