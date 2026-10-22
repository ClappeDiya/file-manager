'use client';

import React from 'react';
import { CreditCard, Users, HardDrive, Check } from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { StatCard } from '@/components/ui/stat-card';
import { ComingSoon } from '@/components/ui/coming-soon';
import { useApi } from '@/lib/use-api';
import { isEnabled } from '@/lib/features';

interface OrgLicense {
  plan: string | null;
  seats: number | null;
  used: number;
  inherited: boolean;
}

const plans = [
  {
    name: 'Starter',
    price: '$0',
    period: 'Free',
    features: ['5 users', '10 GB storage', 'Basic connectors (Local, SFTP)', '7-day audit log', 'Community support'],
  },
  {
    name: 'Business',
    price: '$9.99',
    period: '/user/month',
    features: ['Unlimited users', '1 TB storage', 'All connectors', 'SAML SSO', '90-day audit log', 'Policy engine', 'Approval workflows', 'Email support'],
  },
  {
    name: 'Enterprise',
    price: 'Custom',
    period: 'Contact us',
    features: ['Everything in Business', 'Unlimited storage', 'SAML + SCIM', 'SIEM integration', 'Unlimited audit log', 'Custom policies', 'Dedicated support', 'SLA guarantee'],
  },
];

export default function BillingPage() {
  // The license (plan + seats) is real and set by the parent organization;
  // only payment management is not built yet.
  const { data } = useApi<{ organizations: { id: string; license: OrgLicense }[]; currentOrgId?: string }>('/api/orgs');
  const org = data?.organizations?.find((o) => o.id === data.currentOrgId) ?? data?.organizations?.[0];
  const lic = org?.license;
  const planName = lic?.plan ? lic.plan[0].toUpperCase() + lic.plan.slice(1) : null;

  return (
    <div>
      <PageHeader
        title="Billing"
        description="Manage your subscription, usage, and billing information"
      />

      {!isEnabled('billing') && (
        <ComingSoon
          feature="billing"
          className="mb-6"
          description="Online upgrades, seat changes, invoices and payment methods. Your license below is active; contact sales to change it."
        />
      )}

      {/* License from the organization tree */}
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-4 mb-6">
        <StatCard
          title="Current Plan"
          value={planName ?? '—'}
          icon={CreditCard}
          variant="default"
          subtitle={lic ? (lic.inherited ? 'Inherited from parent organization' : 'Set on this organization') : 'No license assigned'}
        />
        <StatCard title="Seats" value={lic?.seats != null ? `${lic.used} / ${lic.seats}` : lic ? `${lic.used} used` : '—'} icon={Users} variant="default" subtitle="Enrolled machines" />
        <StatCard title="Storage Used" value="—" icon={HardDrive} variant="default" subtitle="Not metered" />
      </div>

      {/* Plans */}
      <h2 className="text-lg font-semibold text-foreground mb-4">Plans</h2>
      <div className="grid grid-cols-1 md:grid-cols-3 gap-4 mb-6">
        {plans.map((plan) => (
          <Card
            key={plan.name}
            className={planName === plan.name ? 'border-primary ring-1 ring-primary' : ''}
          >
            <div className="flex items-center justify-between mb-4">
              <h3 className="text-lg font-bold text-foreground">{plan.name}</h3>
              {planName === plan.name && <Badge variant="info">Current Plan</Badge>}
            </div>
            <div className="mb-4">
              <span className="text-3xl font-bold text-foreground">{plan.price}</span>
              <span className="text-sm text-foreground-secondary"> {plan.period}</span>
            </div>
            <ul className="space-y-2 mb-6">
              {plan.features.map((feature) => (
                <li key={feature} className="flex items-center gap-2 text-sm text-foreground-secondary">
                  <Check size={14} className="text-success shrink-0" />
                  {feature}
                </li>
              ))}
            </ul>
            {planName === plan.name ? (
              <Button variant="outline" className="w-full" disabled>Current Plan</Button>
            ) : (
              <Button
                variant={plan.name === 'Enterprise' ? 'primary' : 'outline'}
                className="w-full"
                disabled={!isEnabled('billing') && plan.name !== 'Enterprise'}
                onClick={plan.name === 'Enterprise' ? () => window.open('https://filemanager.clappe.com/pricing', '_blank', 'noopener') : undefined}
              >
                {plan.name === 'Enterprise' ? 'Contact Sales' : 'Upgrade'}
              </Button>
            )}
          </Card>
        ))}
      </div>

      {/* Billing History */}
      <Card>
        <CardHeader>
          <CardTitle>Billing History</CardTitle>
        </CardHeader>
        <CardContent>
          <div className="py-8 text-center text-sm text-foreground-tertiary">
            No invoices yet. Past invoices will appear here once the first billing cycle closes.
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
