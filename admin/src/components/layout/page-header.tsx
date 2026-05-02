'use client';

import React from 'react';
import { usePathname } from 'next/navigation';
import { cn } from '@/lib/utils/cn';
import { Breadcrumbs, type BreadcrumbItem } from '@/components/ui/breadcrumbs';

interface PageHeaderProps {
  title: string;
  description?: string;
  actions?: React.ReactNode;
  breadcrumbs?: BreadcrumbItem[];
  className?: string;
}

function humanize(segment: string): string {
  return segment
    .split('-')
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(' ');
}

function deriveBreadcrumbs(pathname: string | null, title: string): BreadcrumbItem[] {
  if (!pathname || pathname === '/' || pathname === '/dashboard') return [];
  const segments = pathname.split('/').filter(Boolean);
  // Build cumulative paths so each crumb links back. The last crumb uses the
  // page title (which often differs from the URL slug) and stays unlinked.
  const crumbs: BreadcrumbItem[] = [{ label: 'Dashboard', href: '/dashboard' }];
  let acc = '';
  segments.forEach((seg, idx) => {
    acc += `/${seg}`;
    const isLast = idx === segments.length - 1;
    crumbs.push({ label: isLast ? title : humanize(seg), href: isLast ? undefined : acc });
  });
  return crumbs;
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
