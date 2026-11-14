'use client';

import React from 'react';
import { Clock } from 'lucide-react';
import { cn } from '@/lib/utils/cn';
import { FEATURE_LABELS, type FeatureKey } from '@/lib/features';

interface ComingSoonProps {
  feature: FeatureKey;
  /** What the feature will do, in one or two sentences. */
  description: string;
  /** Optional pointer to what works today. */
  action?: React.ReactNode;
  className?: string;
}

/** Placeholder for a console feature whose backend is not built yet. */
export function ComingSoon({ feature, description, action, className }: ComingSoonProps) {
  return (
    <div
      role="status"
      data-feature={feature}
      className={cn('flex items-start gap-3 rounded-lg border border-dashed border-border p-4', className)}
    >
      <Clock size={18} className="mt-0.5 shrink-0 text-foreground-tertiary" />
      <div className="space-y-1">
        <p className="text-sm font-medium text-foreground">{FEATURE_LABELS[feature]} is coming soon</p>
        <p className="text-sm text-foreground-secondary">{description}</p>
        {action && <div className="pt-1">{action}</div>}
      </div>
    </div>
  );
}
