'use client';

import React from 'react';
import { CreditCard, Users, HardDrive, Check } from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { StatCard } from '@/components/ui/stat-card';

const plans = [
  {
    name: 'Starter',
    price: '$0',
    period: 'Free',
    features: ['5 users', '10 GB storage', 'Basic connectors (Local, SFTP)', '7-day audit log', 'Community support'],
    current: false,
  },
  {
    name: 'Business',
    price: '$29',
    period: '/user/month',
    features: ['Unlimited users', '1 TB storage', 'All connectors', 'SAML SSO', '90-day audit log', 'Policy engine', 'Approval workflows', 'Email support'],
    current: false,
  },
  {
    name: 'Enterprise',
    price: 'Custom',
    period: 'Contact us',
    features: ['Everything in Business', 'Unlimited storage', 'SAML + SCIM', 'SIEM integration', 'Unlimited audit log', 'Custom policies', 'Dedicated support', 'SLA guarantee'],
    current: false,
  },
];

export default function BillingPage() {
  return (
    <div>
      <PageHeader
        title="Billing"
        description="Manage your subscription, usage, and billing information"
      />

      {/* Usage Stats — values populate once a subscription is provisioned. */}
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-4 mb-6">
        <StatCard title="Current Plan" value="—" icon={CreditCard} variant="default" subtitle="No active subscription" />
        <StatCard title="Active Users" value="—" icon={Users} variant="default" />
        <StatCard title="Storage Used" value="—" icon={HardDrive} variant="default" />
      </div>

      {/* Plans */}
      <h2 className="text-lg font-semibold text-foreground mb-4">Plans</h2>
      <div className="grid grid-cols-1 md:grid-cols-3 gap-4 mb-6">
        {plans.map((plan) => (
          <Card
            key={plan.name}
            className={plan.current ? 'border-primary ring-1 ring-primary' : ''}
          >
            <div className="flex items-center justify-between mb-4">
              <h3 className="text-lg font-bold text-foreground">{plan.name}</h3>
              {plan.current && <Badge variant="info">Current Plan</Badge>}
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
            {plan.current ? (
              <Button variant="outline" className="w-full" disabled>Current Plan</Button>
            ) : (
              <Button variant={plan.name === 'Enterprise' ? 'primary' : 'outline'} className="w-full">
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
