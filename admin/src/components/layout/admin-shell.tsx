'use client';

import React, { useEffect } from 'react';
import { useAuthStore } from '@/lib/stores/auth-store';
import { useAdminStore } from '@/lib/stores/admin-store';
import { Sidebar } from './sidebar';
import { Header } from './header';
import { Footer } from './footer';
import { Toaster } from '@/components/ui/toaster';
import { useTranslate } from '@/lib/i18n';

interface AdminShellProps {
  children: React.ReactNode;
}

export function AdminShell({ children }: AdminShellProps) {
  const { isAuthenticated, isValidating, validateSession } = useAuthStore();
  const applyTheme = useAdminStore((s) => s.applyTheme);
  const theme = useAdminStore((s) => s.theme);
  const t = useTranslate();

  useEffect(() => {
    validateSession();
  }, [validateSession]);

  // Apply persisted theme as soon as the shell mounts so a reload no longer
  // resets back to the server-rendered default. When `theme === 'system'`
  // we also live-listen for OS-level light/dark changes.
  useEffect(() => {
    applyTheme();
    if (theme !== 'system' || typeof window === 'undefined') return;
    const media = window.matchMedia('(prefers-color-scheme: dark)');
    const onChange = () => applyTheme();
    media.addEventListener('change', onChange);
    return () => media.removeEventListener('change', onChange);
  }, [applyTheme, theme]);

  if (isValidating) {
    return (
      <div className="flex h-screen items-center justify-center bg-background">
        <div className="text-muted-foreground">Validating session...</div>
      </div>
    );
  }

  if (!isAuthenticated) {
    return (
      <>
        {children}
        <Toaster />
      </>
    );
  }

  return (
    <div className="flex h-screen overflow-hidden bg-background">
      <a
        href="#admin-main"
        className="sr-only focus:not-sr-only focus:absolute focus:top-2 focus:left-2 focus:z-200 focus:rounded-md focus:bg-primary focus:px-3 focus:py-2 focus:text-sm focus:text-primary-foreground"
      >
        {t('shell.skipToContent')}
      </a>
      <Sidebar />
      <div className="flex flex-1 flex-col overflow-hidden">
        <Header />
        <main id="admin-main" tabIndex={-1} className="flex-1 overflow-y-auto p-4 lg:p-6">
          {children}
        </main>
        <Footer />
      </div>
      <Toaster />
    </div>
  );
}
