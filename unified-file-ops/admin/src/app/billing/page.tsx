'use client';

import React from 'react';
import { CreditCard, Users, HardDrive, ArrowUpRight, Check } from 'lucide-react';
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
    current: true,
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

      {/* Usage Stats */}
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-4 mb-6">
        <StatCard title="Current Plan" value="Business" icon={CreditCard} variant="info" subtitle="Billed annually" />
        <StatCard title="Active Users" value="7 / Unlimited" icon={Users} variant="success" />
        <StatCard title="Storage Used" value="245 GB / 1 TB" icon={HardDrive} variant="default" subtitle="24.5% used" />
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
          <div className="space-y-3">
            {[
              { date: 'Mar 1, 2026', amount: '$203.00', status: 'Paid', invoice: 'INV-2026-003' },
              { date: 'Feb 1, 2026', amount: '$203.00', status: 'Paid', invoice: 'INV-2026-002' },
              { date: 'Jan 1, 2026', amount: '$203.00', status: 'Paid', invoice: 'INV-2026-001' },
            ].map((item) => (
              <div key={item.invoice} className="flex items-center justify-between py-2 border-b border-border last:border-0">
                <div>
                  <p className="text-sm font-medium text-foreground">{item.invoice}</p>
                  <p className="text-xs text-foreground-tertiary">{item.date}</p>
                </div>
                <div className="flex items-center gap-3">
                  <span className="text-sm font-medium text-foreground">{item.amount}</span>
                  <Badge variant="success">{item.status}</Badge>
                  <Button variant="ghost" size="sm">
                    <ArrowUpRight size={14} />
                  </Button>
                </div>
              </div>
            ))}
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
