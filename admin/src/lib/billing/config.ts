/**
 * Billing settings read from the environment (on every call, so tests and
 * operators can change them without a rebuild).
 *
 * BILLING_TRIAL_DAYS              Free trial of a first subscription, in days. Default 14 (what the
 *                                 marketing pricing page promises); 0 turns trials off.
 * BILLING_ALLOW_PROMOTION_CODES   Let buyers enter a Stripe promotion code at checkout. Default on;
 *                                 set to 0 / false to hide the field.
 */

/** The trial the marketing pricing page promises. */
export const DEFAULT_TRIAL_DAYS = 14;
/** Stripe accepts trials of up to two years. */
const MAX_TRIAL_DAYS = 730;

export function trialDays(): number {
  const raw = process.env.BILLING_TRIAL_DAYS?.trim();
  if (!raw) return DEFAULT_TRIAL_DAYS;
  const n = Number(raw);
  return Number.isInteger(n) && n >= 0 ? Math.min(n, MAX_TRIAL_DAYS) : DEFAULT_TRIAL_DAYS;
}

export function promotionCodesEnabled(): boolean {
  const raw = process.env.BILLING_ALLOW_PROMOTION_CODES?.trim().toLowerCase();
  return !raw || !['0', 'false', 'no', 'off'].includes(raw);
}
