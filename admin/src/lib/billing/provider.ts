/**
 * Billing provider abstraction: where checkout and the self-service portal
 * live, how webhooks are verified and read, and where invoices come from.
 * Stripe is the implementation shipped (`stripe.ts`); without one the
 * console runs "manual" billing (licenses set by a parent organization).
 */

/** A provider event reduced to what the console acts on. */
export type BillingEvent =
  | { kind: 'checkout_completed'; id: string; orgId: string | null; customerId: string; subscriptionId: string | null; sessionId: string | null }
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
      /** End of the free trial (also set once it has ended). */
      trialEnd: string | null;
      /** The coupon applied to the subscription, if any. */
      discount: Discount | null;
      /** The subscription has its own default payment method. */
      hasPaymentMethod: boolean;
    }
  | { kind: 'trial_will_end'; id: string; orgId: string | null; customerId: string; subscriptionId: string; trialEnd: string | null; hasPaymentMethod: boolean }
  /** The customer's default payment method changed (a card added or removed in the portal). */
  | { kind: 'customer'; id: string; customerId: string; hasPaymentMethod: boolean }
  | { kind: 'invoice'; id: string; customerId: string; paid: boolean; amount: number; currency: string; number: string | null; url: string | null }
  | { kind: 'ignored'; id: string; type: string };

/** A coupon applied to a subscription, reduced to what the billing page shows. */
export interface Discount {
  name: string;
  percentOff: number | null;
  /** Minor units (cents) of `currency`. */
  amountOff: number | null;
  currency: string | null;
  duration: 'once' | 'repeating' | 'forever' | null;
  /** When the discount stops applying (repeating coupons). */
  endsAt: string | null;
}

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
  createCheckout(o: {
    orgId: string;
    priceId: string;
    seats: number;
    customerId: string | null;
    email: string;
    successUrl: string;
    cancelUrl: string;
    /** Free trial in days (no card needed to start); 0 or absent: none. */
    trialDays?: number;
    /** Show the promotion-code field at checkout. */
    allowPromotionCodes?: boolean;
  }): Promise<{ url: string; id: string | null }>;
  /** Close an unfinished checkout session so it can no longer be completed (a newer one replaced it). */
  expireCheckout(sessionId: string): Promise<void>;
  /** The state of a checkout session: still open, completed (a subscription was started) or expired. */
  retrieveCheckout(sessionId: string): Promise<{ status: 'open' | 'complete' | 'expired' }>;
  /** Whether the customer has (or ever had) any subscription at the provider. */
  hasSubscriptions(customerId: string): Promise<boolean>;
  createPortal(o: { customerId: string; returnUrl: string }): Promise<{ url: string }>;
  /** Verify the signature and read the event; throws BillingError(400) on a bad signature. */
  parseWebhook(rawBody: string, headers: Headers, now?: Date): BillingEvent;
  listInvoices(customerId: string): Promise<Invoice[]>;
}
