'use client';

import { create } from 'zustand';

interface AdminUIState {
  sidebarCollapsed: boolean;
  sidebarMobileOpen: boolean;
  currentSection: string;
  theme: 'light' | 'dark' | 'system';

  // Actions
  toggleSidebar: () => void;
  setSidebarCollapsed: (collapsed: boolean) => void;
  toggleMobileSidebar: () => void;
  setCurrentSection: (section: string) => void;
  setTheme: (theme: 'light' | 'dark' | 'system') => void;
}

export const useAdminStore = create<AdminUIState>((set) => ({
  sidebarCollapsed: false,
  sidebarMobileOpen: false,
  currentSection: 'dashboard',
  theme: 'light',

  toggleSidebar: () => set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
  setSidebarCollapsed: (collapsed) => set({ sidebarCollapsed: collapsed }),
  toggleMobileSidebar: () => set((s) => ({ sidebarMobileOpen: !s.sidebarMobileOpen })),
  setCurrentSection: (section) => set({ currentSection: section }),
  setTheme: (theme) => {
    set({ theme });
    if (typeof document !== 'undefined') {
      if (theme === 'system') {
        document.documentElement.removeAttribute('data-theme');
      } else {
        document.documentElement.setAttribute('data-theme', theme);
      }
    }
  },
}));
