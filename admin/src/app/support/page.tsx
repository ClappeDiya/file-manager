'use client';

import React from 'react';
import { HelpCircle, CreditCard, Shield, Lock, ExternalLink } from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';

export default function SupportPage() {
  return (
    <div>
      <PageHeader
        title="Help & Support"
        description="Frequently asked questions, billing information, and how to get help"
      />

      {/* Billing & Payment Information */}
      <Card className="mb-6">
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <CreditCard size={20} />
            Billing &amp; Payment Information
          </CardTitle>
        </CardHeader>
        <CardContent>
          <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
            {/* Statement Descriptor */}
            <div className="space-y-2">
              <h4 className="text-sm font-semibold text-foreground flex items-center gap-2">
                <CreditCard size={16} className="text-primary" />
                Statement Descriptor
              </h4>
              <p className="text-sm text-foreground-secondary">
                Charges from Clapwork will appear as{' '}
                <span className="font-semibold text-foreground bg-background-tertiary px-2 py-0.5 rounded text-xs tracking-wide">
                  CLAPPE.COM
                </span>{' '}
                on your credit card or bank statement. This is the registered billing
                name used by our payment processor.
              </p>
            </div>

            {/* Payment Gateway */}
            <div className="space-y-2">
              <h4 className="text-sm font-semibold text-foreground flex items-center gap-2">
                <ExternalLink size={16} className="text-primary" />
                Payment Gateway
              </h4>
              <p className="text-sm text-foreground-secondary">
                All payments are securely processed through{' '}
                <span className="font-semibold text-foreground">Stripe</span>, an
                industry-leading payment processor. Clapwork never stores your full
                card details — all sensitive payment data is handled entirely by
                Stripe&apos;s PCI-compliant infrastructure.
              </p>
            </div>

            {/* Affiliated Company */}
            <div className="space-y-2">
              <h4 className="text-sm font-semibold text-foreground flex items-center gap-2">
                <HelpCircle size={16} className="text-primary" />
                Affiliated Company
              </h4>
              <p className="text-sm text-foreground-secondary">
                Billing and subscription management is handled by{' '}
                <span className="font-semibold text-foreground">Clappe.com</span>,
                our affiliated billing entity. All invoices, receipts, and
                payment-related correspondence will reference Clappe.com.
              </p>
            </div>
          </div>

          {/* Security Badges */}
          <div className="mt-6 pt-6 border-t border-border">
            <div className="flex flex-wrap items-center gap-4">
              <div className="flex items-center gap-2 bg-background-tertiary rounded-lg px-4 py-2.5 border border-border">
                <Lock size={16} className="text-success" />
                <div className="flex flex-col">
                  <span className="text-xs font-semibold text-foreground">256-bit SSL</span>
                  <span className="text-[10px] text-foreground-tertiary">Encrypted Connection</span>
                </div>
              </div>

              <div className="flex items-center gap-2 bg-background-tertiary rounded-lg px-4 py-2.5 border border-border">
                <Shield size={16} className="text-primary" />
                <div className="flex flex-col">
                  <span className="text-xs font-semibold text-foreground">Processed by Stripe</span>
                  <span className="text-[10px] text-foreground-tertiary">Secure Payments</span>
                </div>
              </div>

              <div className="flex items-center gap-2 bg-background-tertiary rounded-lg px-4 py-2.5 border border-border">
                <Shield size={16} className="text-warning" />
                <div className="flex flex-col">
                  <span className="text-xs font-semibold text-foreground">PCI DSS Compliant</span>
                  <span className="text-[10px] text-foreground-tertiary">Level 1 Certified</span>
                </div>
              </div>
            </div>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
