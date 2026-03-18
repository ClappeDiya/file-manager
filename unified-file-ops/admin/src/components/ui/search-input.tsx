'use client';

import React from 'react';
import { Search } from 'lucide-react';
import { cn } from '@/lib/utils/cn';

interface SearchInputProps extends React.InputHTMLAttributes<HTMLInputElement> {
  onSearch?: (value: string) => void;
}

export function SearchInput({ className, onSearch, onChange, ...props }: SearchInputProps) {
  return (
    <div className="relative">
      <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-foreground-tertiary" />
      <input
        type="search"
        className={cn(
          'flex h-9 w-full rounded-md border border-border bg-background pl-9 pr-3 py-2 text-sm',
          'placeholder:text-foreground-disabled',
          'focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary',
          className
        )}
        onChange={(e) => {
          onChange?.(e);
          onSearch?.(e.target.value);
        }}
        {...props}
      />
    </div>
  );
}
