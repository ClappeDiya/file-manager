import Link from 'next/link';
import { Check } from 'lucide-react';

export type PricingVariant = 'teaser' | 'full';

const plans = [
  {
    name: 'Free',
    price: '$0',
    period: 'forever',
    tagline: 'Everything an individual needs.',
    cta: { label: 'Download', href: '/download', primary: true },
    features: [
      'All 17 protocols',
      'Crash-safe transfers + resume',
      'Real-time bidirectional sync',
      'Local AI assistant (Ollama)',
      'Encryption + vault + audit log',
      'CLI + automation engine',
      'Auto-updates',
      'Community support',
    ],
  },
  {
    name: 'Business',
    price: '$9.99',
    period: '/user/month',
    tagline: 'Shared workspaces + governance for teams.',
    cta: { label: 'Join waitlist', href: '/pricing#waitlist', primary: false },
    badge: 'Coming Q3 2026',
    features: [
      'Everything in Free',
      'Team workspaces (shared connections + configs)',
      'Policy engine (allowed connectors, redaction rules)',
      'Approval workflows for risky operations',
      'Centralized audit log (90-day retention)',
      'SAML SSO',
      'Admin console',
      'Email support',
    ],
  },
  {
    name: 'Enterprise',
    price: 'Custom',
    period: 'annual',
    tagline: 'Scale, compliance, and SLA.',
    cta: { label: 'Contact sales', href: '/contact', primary: false },
    features: [
      'Everything in Business',
      'SCIM provisioning',
      'SIEM integration (Splunk, Datadog)',
      'Unlimited audit retention',
      'Custom connectors',
      'Dedicated support + SLA',
      'Procurement / security reviews',
    ],
  },
];

export function PricingCards({ variant = 'full' }: { variant?: PricingVariant }) {
  return (
    <div className="grid grid-cols-1 md:grid-cols-3 gap-6">
      {plans.map((plan) => (
        <div key={plan.name} className="marketing-card flex flex-col">
          <div className="flex items-center justify-between mb-1">
            <h3 className="text-lg font-bold text-foreground">{plan.name}</h3>
            {plan.badge && <span className="pill">{plan.badge}</span>}
          </div>
          <p className="text-sm text-foreground-secondary mb-5">{plan.tagline}</p>

          <div className="mb-6">
            <span className="text-3xl font-bold text-foreground">{plan.price}</span>
            <span className="text-sm text-foreground-secondary"> {plan.period}</span>
          </div>

          {variant === 'full' && (
            <ul className="space-y-2 mb-6 flex-1">
              {plan.features.map((feature) => (
                <li key={feature} className="flex items-start gap-2 text-sm text-foreground-secondary">
                  <Check size={14} className="text-success shrink-0 mt-0.5" />
                  <span>{feature}</span>
                </li>
              ))}
            </ul>
          )}

          <Link
            href={plan.cta.href}
            className={`mt-auto inline-flex items-center justify-center px-4 py-2 rounded-md text-sm font-medium transition-colors ${
              plan.cta.primary
                ? 'bg-primary text-primary-foreground hover:bg-primary-hover'
                : 'border border-border bg-background text-foreground hover:bg-background-secondary'
            }`}
          >
            {plan.cta.label}
          </Link>
        </div>
      ))}
    </div>
  );
}
