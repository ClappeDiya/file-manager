// Business pricing facts quoted on the pricing page. The admin console's billing code (admin/src/lib/billing)
// sells exactly this: admin/src/__tests__/billing.test.ts fails if the two drift apart.

/** Per user per month, billed monthly. */
export const BUSINESS_MONTHLY_PRICE = '$9.99';
/** Per user per year, billed annually (twelve months less ANNUAL_SAVINGS_PERCENT). */
export const BUSINESS_ANNUAL_PRICE = '$95.90';
export const ANNUAL_SAVINGS_PERCENT = 20;
/** Free trial of a first Business subscription; no card is needed to start it. */
export const TRIAL_DAYS = 14;
