import type { Metadata } from 'next';

export const metadata: Metadata = {
  title: 'Refund Policy',
};

export default function RefundPage() {
  return (
    <div className="container-prose py-16 text-sm text-foreground-secondary leading-relaxed space-y-5">
      <h1 className="marketing-heading">Refund Policy</h1>
      <p className="text-foreground-tertiary text-xs">Last updated: {new Date().toISOString().slice(0, 10)}</p>

      <p>
        The desktop application is free for individuals. There is nothing to refund.
      </p>

      <h2 className="text-xl font-bold text-foreground mt-8">Business plan</h2>
      <p>
        New Business subscriptions include a 14-day free trial. After the trial, your card is charged on a
        rolling monthly or annual basis.
      </p>
      <p>
        If you cancel within 7 days of your first paid charge, email <a href="mailto:billing@clappe.com">billing@clappe.com</a>
        {' '}for a full refund. After 7 days, cancellation takes effect at the end of the current period and we do
        not pro-rate partial periods. Annual plans are not refundable after 30 days from the original purchase
        date except as required by law.
      </p>

      <h2 className="text-xl font-bold text-foreground mt-8">Enterprise plans</h2>
      <p>
        Enterprise contracts are negotiated individually and refunds are governed by the executed agreement.
      </p>

      <h2 className="text-xl font-bold text-foreground mt-8">EU / UK customers</h2>
      <p>
        Where consumer protection law applies, statutory cooling-off periods supersede the terms above. Contact
        billing@clappe.com to invoke your statutory right of withdrawal.
      </p>

      <h2 className="text-xl font-bold text-foreground mt-8">Chargebacks</h2>
      <p>
        Please contact us before filing a chargeback. We resolve almost every billing dispute within 48 hours.
        Chargebacks may result in your account being suspended pending resolution.
      </p>
    </div>
  );
}
