'use client';

import React from 'react';
import { Menu, Bell, Sun, Moon, LogOut, User } from 'lucide-react';
import { useAdminStore } from '@/lib/stores/admin-store';
import { useAuthStore } from '@/lib/stores/auth-store';
import { ROLE_LABELS } from '@/lib/types/auth';
import { Button } from '@/components/ui/button';

export function Header() {
  const { toggleMobileSidebar, theme, setTheme } = useAdminStore();
  const { user, logout } = useAuthStore();

  const toggleTheme = () => {
    setTheme(theme === 'light' ? 'dark' : 'light');
  };

  return (
    <header className="h-14 border-b border-border bg-background-elevated flex items-center justify-between px-4 lg:px-6 shrink-0">
      <div className="flex items-center gap-3">
        <button
          onClick={toggleMobileSidebar}
          className="lg:hidden p-2 rounded-md hover:bg-background-secondary transition-colors"
          aria-label="Toggle sidebar"
        >
          <Menu size={20} />
        </button>
      </div>

      <div className="flex items-center gap-2">
        {/* Notifications */}
        <button
          className="relative p-2 rounded-md hover:bg-background-secondary transition-colors"
          aria-label="Notifications"
        >
          <Bell size={18} className="text-foreground-secondary" />
          <span className="absolute top-1.5 right-1.5 w-2 h-2 bg-error rounded-full" />
        </button>

        {/* Theme toggle */}
        <button
          onClick={toggleTheme}
          className="p-2 rounded-md hover:bg-background-secondary transition-colors"
          aria-label="Toggle theme"
        >
          {theme === 'dark' ? (
            <Sun size={18} className="text-foreground-secondary" />
          ) : (
            <Moon size={18} className="text-foreground-secondary" />
          )}
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
              className="p-2 rounded-md hover:bg-background-secondary transition-colors"
              aria-label="Logout"
              title="Logout"
            >
              <LogOut size={16} className="text-foreground-secondary" />
            </button>
          </div>
        )}
      </div>
    </header>
  );
}
