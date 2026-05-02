'use client';

import React from 'react';
import { Menu, Bell, Sun, Moon, Monitor, LogOut, User } from 'lucide-react';
import { useAdminStore } from '@/lib/stores/admin-store';
import { useAuthStore } from '@/lib/stores/auth-store';
import { ROLE_LABELS } from '@/lib/types/auth';
import { useTranslate } from '@/lib/i18n';

export function Header() {
  const { toggleMobileSidebar, theme, setTheme } = useAdminStore();
  const { user, logout } = useAuthStore();
  const t = useTranslate();

  const cycleTheme = () => {
    setTheme(theme === 'light' ? 'dark' : theme === 'dark' ? 'system' : 'light');
  };
  const ThemeIcon = theme === 'light' ? Sun : theme === 'dark' ? Moon : Monitor;
  const themeLabel = t('shell.themeLabel', { theme });

  return (
    <header className="h-14 border-b border-border bg-background-elevated flex items-center justify-between px-4 lg:px-6 shrink-0">
      <div className="flex items-center gap-3">
        <button
          onClick={toggleMobileSidebar}
          className="lg:hidden h-10 w-10 inline-flex items-center justify-center rounded-md hover:bg-background-secondary transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
          aria-label={t('shell.toggleSidebar')}
        >
          <Menu size={20} aria-hidden="true" />
        </button>
      </div>

      <div className="flex items-center gap-2">
        {/* Notifications */}
        <button
          className="relative h-10 w-10 inline-flex items-center justify-center rounded-md hover:bg-background-secondary transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
          aria-label={t('shell.notifications')}
        >
          <Bell size={18} className="text-foreground-secondary" aria-hidden="true" />
          <span className="absolute top-1.5 right-1.5 w-2 h-2 bg-error rounded-full" aria-hidden="true" />
        </button>

        {/* Theme cycle: light → dark → system */}
        <button
          onClick={cycleTheme}
          className="h-10 w-10 inline-flex items-center justify-center rounded-md hover:bg-background-secondary transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
          aria-label={themeLabel}
          title={themeLabel}
        >
          <ThemeIcon size={18} className="text-foreground-secondary" aria-hidden="true" />
        </button>

        {/* User menu */}
        {user && (
          <div className="flex items-center gap-3 ml-2 pl-3 border-l border-border">
            <div className="hidden sm:block text-right">
              <p className="text-sm font-medium text-foreground">{user.name}</p>
              <p className="text-xs text-foreground-tertiary">{ROLE_LABELS[user.role]}</p>
            </div>
            <div className="w-8 h-8 rounded-full bg-primary/10 flex items-center justify-center">
              <User size={16} className="text-primary" />
            </div>
            <button
              onClick={logout}
              className="h-10 w-10 inline-flex items-center justify-center rounded-md hover:bg-background-secondary transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
              aria-label={t('shell.logout')}
              title={t('shell.logout')}
            >
              <LogOut size={16} className="text-foreground-secondary" aria-hidden="true" />
            </button>
          </div>
        )}
      </div>
    </header>
  );
}
