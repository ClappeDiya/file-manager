'use client';

import React from 'react';
import { cn } from '@/lib/utils/cn';

export interface SelectProps extends React.SelectHTMLAttributes<HTMLSelectElement> {
  label?: string;
  error?: string;
  options: { value: string; label: string }[];
  placeholder?: string;
}

export const Select = React.forwardRef<HTMLSelectElement, SelectProps>(
  ({ className, label, error, options, placeholder, id, ...props }, ref) => {
    const generatedId = React.useId();
    const selectId = id || generatedId;
    const errorId = `${selectId}-error`;

    return (
      <div className="flex flex-col gap-1.5">
        {label && (
          <label htmlFor={selectId} className="text-sm font-medium text-foreground">
            {label}
          </label>
        )}
        <select
          ref={ref}
          id={selectId}
          className={cn(
            'flex h-9 w-full rounded-md border bg-background px-3 py-2 text-sm',
            'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary focus-visible:border-border-focus',
            'disabled:cursor-not-allowed disabled:opacity-50',
            error ? 'border-border-error' : 'border-border',
            className
          )}
          {...props}
          aria-invalid={error ? true : props["aria-invalid"]}
          aria-describedby={[props["aria-describedby"], error ? errorId : null].filter(Boolean).join(" ") || undefined}
        >
          {placeholder && <option value="">{placeholder}</option>}
          {options.map((opt) => (
            <option key={opt.value} value={opt.value}>
              {opt.label}
            </option>
          ))}
        </select>
        {error && <p id={errorId} role="alert" className="text-xs text-error-foreground">{error}</p>}
      </div>
    );
  }
);

Select.displayName = 'Select';
