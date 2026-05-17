import type { Metadata } from 'next';
import { PricingCards } from '@/components/pricing-cards';
import { WaitlistForm } from '@/components/waitlist-form';

export const metadata: Metadata = {
  title: 'Pricing',
  description: 'Free for individuals. Business $9.99/user/month (coming Q3). Enterprise custom.',
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
    q: 'Why $9.99/user/month?',
    a: 'It is the lowest sustainable price that lets us keep the desktop free, fund development, and run a managed SaaS without venture-scale unit economics. Annual billing saves 20%; non-profits get 50% off; students get it free.',
  },
  {
    q: 'How does Business billing work?',
    a: 'Per-seat monthly or annual. You add seats in the admin console; we adjust your Stripe subscription automatically. Cancel anytime; cancellation takes effect at the end of the current period.',
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
    a: 'Yes — 14 days, no credit card required at signup. We add the card before the trial converts.',
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
