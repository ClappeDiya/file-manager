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

/**
 * Whether a payment method is on file for the subscription: true when the subscription or the customer has a
 * default, false when the subscription is known to have none, null when no event has said yet.
 */
export function hasPaymentMethod(a: { subPaymentMethod: boolean | null; customerPaymentMethod: boolean | null }): boolean | null {
  if (a.subPaymentMethod || a.customerPaymentMethod) return true;
  return a.subPaymentMethod === false ? false : null;
}

/** What happens when the trial ends, in words (billing page banner, notices). */
export function trialOutcome(pm: boolean | null): string {
  if (pm === true) return `The trial then converts to a paid subscription.`;
  if (pm === false) return `No payment method is on file: add one in the billing portal before then to keep the subscription; otherwise the subscription is cancelled and the organization returns to the free plan.`;
  return `Check in the billing portal that a payment method is on file; without one the subscription is cancelled when the trial ends and the organization returns to the free plan.`;
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
