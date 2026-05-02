'use client';

import React from 'react';
import Link from 'next/link';
import { useTranslate } from '@/lib/i18n';

export function Footer() {
  const t = useTranslate();
  const year = new Date().getFullYear();
  const items = [
    { key: 'privacy', href: '/privacy' },
    { key: 'terms', href: '/terms' },
    { key: 'status', href: '/status' },
    { key: 'support', href: '/support' },
  ] as const;
  return (
    <footer
      className="border-t border-border bg-background-elevated px-4 lg:px-6 py-3 flex flex-col-reverse sm:flex-row sm:items-center sm:justify-between gap-2 text-xs text-foreground-tertiary shrink-0"
      role="contentinfo"
    >
      <p>{t('footer.copyright', { year })}</p>
      <nav aria-label="Footer">
        <ul className="flex flex-wrap items-center gap-x-4 gap-y-1">
          {items.map((item) => (
            <li key={item.href}>
              <Link
                href={item.href}
                className="hover:text-foreground transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary rounded-sm"
              >
                {t(`footer.${item.key}`)}
              </Link>
            </li>
          ))}
        </ul>
      </nav>
    </footer>
  );
}
