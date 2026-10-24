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
