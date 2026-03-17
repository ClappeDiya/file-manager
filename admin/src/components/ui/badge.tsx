'use client';

import React from 'react';
import { cn } from '@/lib/utils/cn';

export interface BadgeProps extends React.HTMLAttributes<HTMLSpanElement> {
  variant?: 'default' | 'success' | 'warning' | 'error' | 'info' | 'outline';
}

export function Badge({ className, variant = 'default', ...props }: BadgeProps) {
  const variants = {
    default: 'bg-background-tertiary text-foreground-secondary',
    success: 'bg-success-bg text-success-foreground',
    warning: 'bg-warning-bg text-warning-foreground',
    error: 'bg-error-bg text-error-foreground',
    info: 'bg-info-bg text-info-foreground',
    outline: 'border border-border text-foreground-secondary bg-transparent',
  };

  return (
    <span
      className={cn(
        'inline-flex items-center rounded-full px-2.5 py-0.5 text-xs font-medium',
        variants[variant],
        className
      )}
      {...props}
    />
  );
}
