'use client';

import React from 'react';
import { cn } from '@/lib/utils/cn';

export interface InputProps extends React.InputHTMLAttributes<HTMLInputElement> {
  label?: string;
  error?: string;
  hint?: string;
}

export const Input = React.forwardRef<HTMLInputElement, InputProps>(
  ({ className, label, error, hint, id, ...props }, ref) => {
    const inputId = id || label?.toLowerCase().replace(/\s+/g, '-');

    return (
      <div className="flex flex-col gap-1.5">
        {label && (
          <label htmlFor={inputId} className="text-sm font-medium text-foreground">
            {label}
          </label>
        )}
        <input
          ref={ref}
          id={inputId}
          className={cn(
            'flex h-9 w-full rounded-md border bg-background px-3 py-2 text-sm',
            'placeholder:text-foreground-disabled',
            'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:border-border-focus',
            'disabled:cursor-not-allowed disabled:opacity-50',
            error ? 'border-border-error' : 'border-border',
            className
          )}
          {...props}
        />
        {error && <p className="text-xs text-error">{error}</p>}
        {hint && !error && <p className="text-xs text-foreground-tertiary">{hint}</p>}
      </div>
    );
  }
);

Input.displayName = 'Input';
