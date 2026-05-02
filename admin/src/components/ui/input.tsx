'use client';

import React from 'react';
import { Input as SharedInput, type InputProps as SharedInputProps } from '@ufop/ui-components';
import { cn } from '@/lib/utils/cn';

/**
 * Admin Input — wraps the shared `@ufop/ui-components` Input with the admin's
 * label / error / hint affordances. Bare-input styles, focus rings, and
 * `aria-invalid` come from the shared package; the admin layer adds:
 *  - a visible <label> tied to the input
 *  - field-level error message with `role="alert"` + `aria-describedby`
 *  - hint text that swaps out when an error is present
 */
export interface InputProps extends Omit<SharedInputProps, 'error'> {
  label?: string;
  error?: string;
  hint?: string;
}

export const Input = React.forwardRef<HTMLInputElement, InputProps>(
  ({ className, label, error, hint, id, ...props }, ref) => {
    const generatedId = React.useId();
    const inputId = id || (label ? `${label.toLowerCase().replace(/\s+/g, '-')}-${generatedId}` : generatedId);
    const helperId = `${inputId}-helper`;

    return (
      <div className="flex flex-col gap-1.5">
        {label && (
          <label htmlFor={inputId} className="text-sm font-medium text-foreground">
            {label}
          </label>
        )}
        <SharedInput
          ref={ref}
          id={inputId}
          error={Boolean(error)}
          aria-describedby={(error || hint) ? helperId : undefined}
          className={cn(className)}
          {...props}
        />
        {error && <p id={helperId} className="text-xs text-error" role="alert">{error}</p>}
        {hint && !error && <p id={helperId} className="text-xs text-foreground-tertiary">{hint}</p>}
      </div>
    );
  },
);

Input.displayName = 'Input';
