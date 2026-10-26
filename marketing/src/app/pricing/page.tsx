import type { Metadata } from 'next';
import { PricingCards } from '@/components/pricing-cards';
import { WaitlistForm } from '@/components/waitlist-form';
import { ANNUAL_SAVINGS_PERCENT, BUSINESS_ANNUAL_PERIOD, BUSINESS_ANNUAL_PRICE, BUSINESS_MONTHLY_PERIOD, BUSINESS_MONTHLY_PRICE, BUSINESS_UNIT, TRIAL_DAYS } from '@/lib/pricing';

export const metadata: Metadata = {
  title: 'Pricing',
  description: `Free for individuals. Business ${BUSINESS_MONTHLY_PRICE}${BUSINESS_MONTHLY_PERIOD} or ${BUSINESS_ANNUAL_PRICE}${BUSINESS_ANNUAL_PERIOD}, with a ${TRIAL_DAYS}-day free trial (coming Q3). Enterprise custom.`,
};

const faq = [
  {
    q: 'Is the desktop app really free forever?',
    a: 'Yes. All file-management features — every connector, every transfer, sync, encryption, local AI — are and will remain free for individuals. We only charge teams for the coordination layer (shared workspaces, audit, SSO, approval workflows).',
  },
  {
    q: 'What is Business actually for?',
    a: 'Teams that need to share file-manager connections, enforce policies (e.g. "S3 buckets in eu-west-1 only"), require approval before risky operations, or keep a centralized audit log for compliance. None of that matters to a solo user; all of it matters once you have 5+ people moving files together.',
  },
  {
    q: `Why ${BUSINESS_MONTHLY_PRICE}${BUSINESS_MONTHLY_PERIOD}?`,
    a: `It is the lowest sustainable price that lets us keep the desktop free, fund development, and run a managed SaaS without venture-scale unit economics. Annual billing is ${BUSINESS_ANNUAL_PRICE} per ${BUSINESS_UNIT} per year, ${ANNUAL_SAVINGS_PERCENT}% less than twelve months at ${BUSINESS_MONTHLY_PRICE}. If you have a promotion code, enter it at checkout.`,
  },
  {
    q: 'How does Business billing work?',
    a: `Per seat, where a seat is one enrolled ${BUSINESS_UNIT} (not a person), monthly or annual: pick one on the Billing page of the admin console and pay through Stripe Checkout. Seats, the payment method and cancellation are managed in the Stripe billing portal linked from the same page. Cancellation takes effect at the end of the current period.`,
  },
  {
    q: 'When does Business launch?',
    a: 'Q3 2026. The waitlist is for early-adopter pricing and to surface design feedback before we lock the v1 surface. We will not auto-charge you — joining the waitlist costs nothing.',
  },
  {
    q: 'What about offline use?',
    a: 'The desktop app works fully offline; no license check, no phone-home. Business license is verified once every 7 days against api.filemanager.clappe.com; if offline for 30 days, your team features gracefully degrade to free-tier. Nothing ever locks you out of files you have already downloaded.',
  },
  {
    q: 'Do you offer a free trial of Business?',
    a: `Yes — ${TRIAL_DAYS} days, no credit card required to start. The admin console shows how many days are left and three days before the trial ends we notify your organization’s channels (email, Slack, Teams, webhooks). Add a payment method in the billing portal before then to keep Business; if you do not, the subscription is cancelled when the trial ends and your organization returns to the free plan. Each organization gets one trial.`,
  },
  {
    q: 'Is this open source?',
    a: 'Source-available under PolyForm Shield 1.0.0 — read, modify, redistribute freely, except for building a competing product. Not OSI-approved open source; it is a deliberate trade-off so we can stay independent without VC.',
  },
];

export default function PricingPage() {
  return (
    <div className="container-wide py-16">
      <header className="max-w-3xl mx-auto text-center mb-12">
        <h1 className="marketing-heading">Pricing</h1>
        <p className="marketing-subheading mt-4">
          Free desktop forever. Pay only when your team needs to coordinate.
        </p>
      </header>

      <PricingCards variant="full" />

      <div className="mt-16 max-w-3xl mx-auto">
        <WaitlistForm />
      </div>

      <section className="mt-20 max-w-3xl mx-auto">
        <h2 className="text-2xl font-bold text-foreground mb-6">Frequently asked</h2>
        <dl className="space-y-6">
          {faq.map((item) => (
            <div key={item.q} className="border-b border-border pb-6 last:border-b-0">
              <dt className="font-semibold text-foreground mb-2">{item.q}</dt>
              <dd className="text-sm text-foreground-secondary leading-relaxed">{item.a}</dd>
            </div>
          ))}
        </dl>
      </section>
    </div>
  );
}
