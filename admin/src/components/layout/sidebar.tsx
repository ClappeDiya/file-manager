'use client';

import React from 'react';
import Link from 'next/link';
import { usePathname } from 'next/navigation';
import {
  LayoutDashboard,
  Users,
  Monitor,
  Shield,
  CheckSquare,
  ScrollText,
  Plug,
  Brain,
  CreditCard,
  FolderSync,
  ChevronLeft,
  ChevronRight,
  Settings,
} from 'lucide-react';
import { cn } from '@/lib/utils/cn';
import { useAdminStore } from '@/lib/stores/admin-store';
import { useAuthStore } from '@/lib/stores/auth-store';
import type { Permission } from '@/lib/types/auth';
import { useTranslate } from '@/lib/i18n';

interface NavItem {
  id: string;
  /** Translation key under `nav.*` — resolved at render time so locale changes update labels live. */
  i18nKey: string;
  href: string;
  icon: React.ElementType;
  requiredPermission?: Permission;
  badge?: number;
}

const navItems: NavItem[] = [
  { id: 'dashboard', i18nKey: 'nav.dashboard', href: '/dashboard', icon: LayoutDashboard },
  { id: 'users', i18nKey: 'nav.users', href: '/users', icon: Users, requiredPermission: 'users.read' },
  { id: 'devices', i18nKey: 'nav.devices', href: '/devices', icon: Monitor, requiredPermission: 'devices.read' },
  { id: 'policies', i18nKey: 'nav.policies', href: '/policies', icon: Shield, requiredPermission: 'policies.read' },
  { id: 'approvals', i18nKey: 'nav.approvals', href: '/approvals', icon: CheckSquare, requiredPermission: 'approvals.read' },
  { id: 'audit', i18nKey: 'nav.audit', href: '/audit', icon: ScrollText, requiredPermission: 'audit.read' },
  { id: 'connectors', i18nKey: 'nav.connectors', href: '/connectors', icon: Plug, requiredPermission: 'connectors.read' },
  { id: 'workspaces', i18nKey: 'nav.workspaces', href: '/workspaces', icon: FolderSync, requiredPermission: 'workspaces.read' },
  { id: 'ai-governance', i18nKey: 'nav.aiGovernance', href: '/ai-governance', icon: Brain, requiredPermission: 'ai_governance.read' },
  { id: 'billing', i18nKey: 'nav.billing', href: '/billing', icon: CreditCard, requiredPermission: 'billing.read' },
];

export function Sidebar() {
  const pathname = usePathname();
  const { sidebarCollapsed, toggleSidebar, sidebarMobileOpen, toggleMobileSidebar } = useAdminStore();
  const checkPermission = useAuthStore((s) => s.checkPermission);
  const t = useTranslate();

  // Render restricted nav items as visible-but-disabled with a tooltip rather
  // than hiding them silently. The audit caught that admins with lower roles
  // could not tell whether a feature exists at all vs. is just gated.
  const itemsWithPermission = navItems.map((item) => ({
    item,
    allowed: !item.requiredPermission || checkPermission(item.requiredPermission),
  }));

  return (
    <>
      {/* Mobile overlay */}
      {sidebarMobileOpen && (
        <div
          className="fixed inset-0 z-40 bg-black/50 lg:hidden"
          onClick={toggleMobileSidebar}
        />
      )}

      <aside
        className={cn(
          'fixed top-0 left-0 z-50 h-full bg-sidebar-bg border-r border-sidebar-border flex flex-col transition-all duration-slow',
          sidebarCollapsed ? 'w-16' : 'w-64',
          'lg:relative',
          sidebarMobileOpen ? 'translate-x-0' : '-translate-x-full lg:translate-x-0'
        )}
      >
        {/* Logo area */}
        <div className={cn(
          'flex items-center h-14 border-b border-sidebar-border px-4',
          sidebarCollapsed ? 'justify-center' : 'justify-between'
        )}>
          {!sidebarCollapsed && (
            <Link href="/dashboard" className="flex items-center gap-2">
              <div className="w-8 h-8 rounded-lg bg-primary flex items-center justify-center">
                <span className="text-primary-foreground font-bold text-sm">U</span>
              </div>
              <span className="font-semibold text-sidebar-text-active text-sm">UFOP Admin</span>
            </Link>
          )}
          {sidebarCollapsed && (
            <div className="w-8 h-8 rounded-lg bg-primary flex items-center justify-center">
              <span className="text-primary-foreground font-bold text-sm">U</span>
            </div>
          )}
        </div>

        {/* Navigation */}
        <nav className="flex-1 overflow-y-auto py-3 px-2">
          <ul className="space-y-0.5">
            {itemsWithPermission.map(({ item, allowed }) => {
              const isActive = allowed && (pathname === item.href || pathname?.startsWith(item.href + '/'));
              const Icon = item.icon;
              const label = t(item.i18nKey);
              const lockedTitle = `${label} — requires ${item.requiredPermission} permission`;
              const tooltip = sidebarCollapsed
                ? allowed ? label : lockedTitle
                : allowed ? undefined : lockedTitle;

              const inner = (
                <>
                  <Icon size={18} className="shrink-0" aria-hidden="true" />
                  {!sidebarCollapsed && (
                    <>
                      <span className="flex-1">{label}</span>
                      {!allowed && (
                        <span className="text-[10px] uppercase tracking-wide text-sidebar-text/60">
                          {t('shell.locked')}
                        </span>
                      )}
                      {allowed && item.badge !== undefined && item.badge > 0 && (
                        <span className="inline-flex items-center justify-center min-w-[1.25rem] h-5 rounded-full bg-error text-white text-xs px-1">
                          {item.badge}
                        </span>
                      )}
                    </>
                  )}
                </>
              );

              const baseClass = cn(
                'flex items-center gap-3 rounded-md px-3 py-2 text-sm font-medium transition-colors',
                sidebarCollapsed && 'justify-center px-2',
              );

              return (
                <li key={item.id}>
                  {allowed ? (
                    <Link
                      href={item.href}
                      onClick={() => sidebarMobileOpen && toggleMobileSidebar()}
                      className={cn(
                        baseClass,
                        isActive
                          ? 'bg-primary/10 text-primary'
                          : 'text-sidebar-text hover:bg-background-tertiary hover:text-sidebar-text-active',
                      )}
                      title={tooltip}
                    >
                      {inner}
                    </Link>
                  ) : (
                    <div
                      role="link"
                      aria-disabled="true"
                      tabIndex={0}
                      className={cn(baseClass, 'cursor-not-allowed text-sidebar-text/50')}
                      title={tooltip}
                    >
                      {inner}
                    </div>
                  )}
                </li>
              );
            })}
          </ul>
        </nav>

        {/* Collapse toggle */}
        <div className="border-t border-sidebar-border p-2">
          <button
            onClick={toggleSidebar}
            className="w-full flex items-center justify-center gap-2 rounded-md px-3 py-2 text-sm text-sidebar-text hover:bg-background-tertiary transition-colors"
            title={sidebarCollapsed ? 'Expand sidebar' : 'Collapse sidebar'}
          >
            {sidebarCollapsed ? <ChevronRight size={16} /> : <ChevronLeft size={16} />}
            {!sidebarCollapsed && <span>Collapse</span>}
          </button>
        </div>
      </aside>
    </>
  );
}
