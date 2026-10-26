/** Pure helpers for showing a trial and a discount (also used by the billing page in the browser). */
import type { Discount } from './provider';

const DAY_MS = 86_400_000;

/** Whole days left until `trialEnd` (rounded up; 0 once it has passed); null without a trial. */
export function trialDaysLeft(trialEnd: string | null | undefined, at: Date = new Date()): number | null {
  if (!trialEnd) return null;
  const end = Date.parse(trialEnd);
  if (Number.isNaN(end)) return null;
  return Math.max(0, Math.ceil((end - at.getTime()) / DAY_MS));
}

/** "soon" / "today" / "in 1 day" / "in 3 days", for the audit entry, the notice and the billing page banner. */
export function trialEndsPhrase(left: number | null): string {
  if (left === null) return 'soon';
  if (left <= 0) return 'today';
  return `in ${left} day${left === 1 ? '' : 's'}`;
}

/**
 * Whether a payment method is on file: true when the subscription or the customer has a default; false only when
 * both are known to have none; null otherwise. A subscription without a default says nothing about the customer
 * (a card added in the billing portal becomes the customer's default and arrives as `customer.updated`), so
 * until that event has been received the answer stays unknown rather than claiming there is no card.
 */
export function hasPaymentMethod(a: { subPaymentMethod: boolean | null; customerPaymentMethod: boolean | null }): boolean | null {
  if (a.subPaymentMethod || a.customerPaymentMethod) return true;
  return a.subPaymentMethod === false && a.customerPaymentMethod === false ? false : null;
}

/** What happens when the trial ends, in words (billing page banner, notices). */
export function trialOutcome(pm: boolean | null): string {
  if (pm === true) return `The trial then converts to a paid subscription.`;
  if (pm === false) return `No payment method is on file: add one in the billing portal before then to keep the subscription; otherwise the subscription is cancelled and the organization returns to the free plan.`;
  return `Check in the billing portal that a payment method is on file; without one the subscription is cancelled when the trial ends and the organization returns to the free plan.`;
}

/** The trial outcome in one clause for the audit trail (same three cases as `trialOutcome`). */
export function trialOutcomeShort(pm: boolean | null): string {
  if (pm === true) return 'a payment method is on file, so it converts to a paid subscription';
  if (pm === false) return 'add a payment method in the billing portal to keep the subscription';
  return 'check in the billing portal that a payment method is on file to keep the subscription';
}

/**
 * The line above the plans: the free trial (when this organization gets one)
 * and the promotion-code field (when checkout shows it,
 * `BILLING_ALLOW_PROMOTION_CODES`); null when neither applies.
 */
export function checkoutNote(trial: { days: number; eligible: boolean }, promotionCodes: boolean): string | null {
  const parts: string[] = [];
  if (trial.eligible && trial.days > 0) parts.push(`Your first subscription starts with a ${trial.days}-day free trial; no card is needed to start.`);
  if (promotionCodes) parts.push('Have a promotion code? Enter it at checkout.');
  return parts.length ? parts.join(' ') : null;
}

/** "20% off" / "$5.00 off", for the billing page. */
export function discountAmount(d: Discount): string {
  if (d.percentOff != null) return `${Number(d.percentOff.toFixed(2))}% off`;
  if (d.amountOff != null) return `${new Intl.NumberFormat(undefined, { style: 'currency', currency: d.currency ?? 'USD' }).format(d.amountOff / 100)} off`;
  return 'Discount';
}

/** How long the discount lasts, in words. */
export function discountDuration(d: Discount): string {
  if (d.duration === 'forever') return 'every invoice';
  if (d.duration === 'once') return 'the first invoice';
  if (d.duration === 'repeating') return d.endsAt ? `until ${new Date(d.endsAt).toLocaleDateString()}` : 'for a limited time';
  return d.endsAt ? `until ${new Date(d.endsAt).toLocaleDateString()}` : '';
}
