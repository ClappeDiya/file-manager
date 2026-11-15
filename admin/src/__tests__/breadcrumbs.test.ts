import { readdirSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { deriveBreadcrumbs } from '@/lib/breadcrumbs';

describe('page breadcrumbs', () => {
  it('keeps route groups without index pages as readable labels', () => {
    for (const route of ['/settings/security', '/integrations/webhooks']) {
      const crumbs = deriveBreadcrumbs(route, 'Current page');
      expect(crumbs[1].label).toBeTruthy();
      expect(crumbs[1].href).toBeUndefined();
      expect(crumbs.at(-1)).toEqual({ label: 'Current page', href: undefined });
    }
  });
  it('links existing fleet and machine list ancestors', () => {
    expect(deriveBreadcrumbs('/fleet/machines/machine-id', 'Machine').map(c => c.href)).toEqual(['/dashboard', '/fleet', '/fleet/machines', undefined]);
  });
  it('never generates a link to an absent page for any current route', () => {
    const root = fileURLToPath(new URL('../app', import.meta.url));
    const pages = readdirSync(root, { recursive: true }).map(String).filter(p => p.endsWith('page.tsx'));
    expect(pages.length).toBeGreaterThan(30);
    for (const page of pages) {
      const route = '/' + page.replace(/\/?page\.tsx$/, '');
      for (const crumb of deriveBreadcrumbs(route, 'Page')) {
        if (crumb.href) expect(existsSync(join(root, crumb.href, 'page.tsx')), `${route} links to ${crumb.href}`).toBe(true);
      }
    }
  });
});
