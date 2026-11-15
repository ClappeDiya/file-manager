import type { BreadcrumbItem } from '@/components/ui/breadcrumbs';

// These route groups contain pages but have no index page.
const NON_NAVIGABLE_PARENTS = new Set(['/settings', '/integrations', '/invite']);

function humanize(segment: string): string {
  return segment
    .split('-')
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(' ');
}

export function deriveBreadcrumbs(pathname: string | null, title: string): BreadcrumbItem[] {
  if (!pathname || pathname === '/' || pathname === '/dashboard') return [];
  const segments = pathname.split('/').filter(Boolean);
  // Link only real page ancestors. Route groups without landing pages remain labels.
  const crumbs: BreadcrumbItem[] = [{ label: 'Dashboard', href: '/dashboard' }];
  let acc = '';
  segments.forEach((seg, idx) => {
    acc += `/${seg}`;
    const isLast = idx === segments.length - 1;
    crumbs.push({ label: isLast ? title : humanize(seg), href: isLast || NON_NAVIGABLE_PARENTS.has(acc) ? undefined : acc });
  });
  return crumbs;
}

