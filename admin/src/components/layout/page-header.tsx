'use client';

import React from 'react';
import { usePathname } from 'next/navigation';
import { cn } from '@/lib/utils/cn';
import { deriveBreadcrumbs } from '@/lib/breadcrumbs';
import { Breadcrumbs, type BreadcrumbItem } from '@/components/ui/breadcrumbs';

interface PageHeaderProps {
  title: string;
  description?: string;
  actions?: React.ReactNode;
  breadcrumbs?: BreadcrumbItem[];
  className?: string;
}

export function PageHeader({ title, description, actions, breadcrumbs, className }: PageHeaderProps) {
  const pathname = usePathname();
  const derived = breadcrumbs ?? deriveBreadcrumbs(pathname, title);
  return (
    <div className={cn('flex flex-col gap-4 mb-6', className)}>
      {derived.length > 0 && <Breadcrumbs items={derived} />}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold text-foreground">{title}</h1>
          {description && (
            <p className="text-sm text-foreground-secondary mt-1">{description}</p>
          )}
        </div>
        {actions && <div className="flex items-center gap-2">{actions}</div>}
      </div>
    </div>
  );
}
