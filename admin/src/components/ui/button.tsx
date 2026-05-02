'use client';

import React from 'react';
import { Button as SharedButton, type ButtonProps as SharedButtonProps } from '@ufop/ui-components';

/**
 * Admin Button — thin compatibility wrapper around `@ufop/ui-components`.
 *
 * The shared package owns the actual styles/states/aria-busy. This file maps
 * the admin's legacy variant names (`primary`, `danger`) to the shared package's
 * (`default`, `destructive`) so existing admin call sites keep working without
 * a sweeping rename. New code should import from `@ufop/ui-components` directly.
 */
type LegacyVariant = 'primary' | 'danger';
type SharedVariant = NonNullable<SharedButtonProps['variant']>;

export interface ButtonProps
  extends Omit<SharedButtonProps, 'variant'> {
  variant?: SharedVariant | LegacyVariant;
}

const VARIANT_MAP: Record<LegacyVariant, SharedVariant> = {
  primary: 'default',
  danger: 'destructive',
};

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ variant = 'primary', ...props }, ref) => {
    const mapped = (VARIANT_MAP as Record<string, SharedVariant>)[variant] ?? (variant as SharedVariant);
    return <SharedButton ref={ref} variant={mapped} {...props} />;
  },
);

Button.displayName = 'Button';
