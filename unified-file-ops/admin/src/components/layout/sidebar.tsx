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
import { Badge } from '@/components/ui/badge';
import type { Permission } from '@/lib/types/auth';

interface NavItem {
  id: string;
  label: string;
  href: string;
  icon: React.ElementType;
  requiredPermission?: Permission;
  badge?: number;
}

const navItems: NavItem[] = [
  { id: 'dashboard', label: 'Dashboard', href: '/dashboard', icon: LayoutDashboard },
  { id: 'users', label: 'Users & Roles', href: '/users', icon: Users, requiredPermission: 'users.read' },
  { id: 'devices', label: 'Devices', href: '/devices', icon: Monitor, requiredPermission: 'devices.read' },
  { id: 'policies', label: 'Policies', href: '/policies', icon: Shield, requiredPermission: 'policies.read' },
  { id: 'approvals', label: 'Approvals', href: '/approvals', icon: CheckSquare, requiredPermission: 'approvals.read' },
  { id: 'audit', label: 'Audit Log', href: '/audit', icon: ScrollText, requiredPermission: 'audit.read' },
  { id: 'connectors', label: 'Connectors', href: '/connectors', icon: Plug, requiredPermission: 'connectors.read' },
  { id: 'workspaces', label: 'Workspaces', href: '/workspaces', icon: FolderSync, requiredPermission: 'workspaces.read' },
  { id: 'ai-governance', label: 'AI Governance', href: '/ai-governance', icon: Brain, requiredPermission: 'ai_governance.read' },
  { id: 'billing', label: 'Billing', href: '/billing', icon: CreditCard, requiredPermission: 'billing.read' },
];

export function Sidebar() {
  const pathname = usePathname();
  const { sidebarCollapsed, toggleSidebar, sidebarMobileOpen, toggleMobileSidebar } = useAdminStore();
  const checkPermission = useAuthStore((s) => s.checkPermission);

  const filteredItems = navItems.filter(
    (item) => !item.requiredPermission || checkPermission(item.requiredPermission)
  );

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
              <div className="flex flex-col">
                <span className="font-semibold text-sidebar-text-active text-sm leading-tight">UFOP Admin</span>
                <Badge variant="info" className="text-[9px] px-1.5 py-0 mt-0.5 w-fit">Enterprise</Badge>
              </div>
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
            {filteredItems.map((item) => {
              const isActive = pathname === item.href || pathname?.startsWith(item.href + '/');
              const Icon = item.icon;

              return (
                <li key={item.id}>
                  <Link
                    href={item.href}
                    onClick={() => sidebarMobileOpen && toggleMobileSidebar()}
                    className={cn(
                      'flex items-center gap-3 rounded-md px-3 py-2 text-sm font-medium transition-colors',
                      isActive
                        ? 'bg-primary/10 text-primary'
                        : 'text-sidebar-text hover:bg-background-tertiary hover:text-sidebar-text-active',
                      sidebarCollapsed && 'justify-center px-2'
                    )}
                    title={sidebarCollapsed ? item.label : undefined}
                  >
                    <Icon size={18} className="shrink-0" />
                    {!sidebarCollapsed && (
                      <>
                        <span className="flex-1">{item.label}</span>
                        {item.badge !== undefined && item.badge > 0 && (
                          <span className="inline-flex items-center justify-center min-w-[1.25rem] h-5 rounded-full bg-error text-white text-xs px-1">
                            {item.badge}
                          </span>
                        )}
                      </>
                    )}
                  </Link>
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
