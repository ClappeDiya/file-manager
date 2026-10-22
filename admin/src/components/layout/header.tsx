'use client';

import React from 'react';
import Link from 'next/link';
import { useRouter } from 'next/navigation';
import { Menu, Bell, Sun, Moon, Monitor, LogOut, User, Building2 } from 'lucide-react';
import { useToastStore } from '@/lib/stores/toast-store';
import { useAdminStore } from '@/lib/stores/admin-store';
import { useAuthStore } from '@/lib/stores/auth-store';
import { ROLE_LABELS } from '@/lib/types/auth';
import { useTranslate } from '@/lib/i18n';

export function Header() {
  const { toggleMobileSidebar, theme, setTheme } = useAdminStore();
  const { user, logout, organization, organizations, switchOrg } = useAuthStore();
  const router = useRouter();
  const notify = useToastStore((s) => s.notify);
  const t = useTranslate();

  const onSwitch = async (orgId: string) => {
    try {
      await switchOrg(orgId);
      router.refresh();
    } catch (e) {
      notify(e instanceof Error ? e.message : String(e), 'error');
    }
  };

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
        {/* Organization switcher: every organization the user can reach (own memberships, groups and delegated children). */}
        {organization && organizations.length > 1 && (
          <label className="hidden md:flex items-center gap-2 text-sm text-foreground-secondary">
            <Building2 size={16} aria-hidden="true" />
            <span className="sr-only">{t('shell.organization')}</span>
            <select
              value={organization.id}
              onChange={(e) => void onSwitch(e.target.value)}
              className="h-9 max-w-[16rem] rounded-md border border-border bg-background px-2 text-sm text-foreground"
            >
              {organizations.map((o) => (
                <option key={o.id} value={o.id}>
                  {o.path}
                </option>
              ))}
            </select>
          </label>
        )}
        {organization && organizations.length <= 1 && <span className="hidden md:inline text-sm text-foreground-secondary">{organization.name}</span>}

        {/* Notifications */}
        <Link
          href="/alerts"
          className="relative h-10 w-10 inline-flex items-center justify-center rounded-md hover:bg-background-secondary transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
          aria-label={t('shell.notifications')}
        >
          <Bell size={18} className="text-foreground-secondary" aria-hidden="true" />
        </Link>

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
            <Link href="/account" aria-label={t('nav.account')} className="w-8 h-8 rounded-full bg-primary/10 flex items-center justify-center">
              <User size={16} className="text-primary" />
            </Link>
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
