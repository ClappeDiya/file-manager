'use client';

import { create } from 'zustand';
import { persist, createJSONStorage } from 'zustand/middleware';

export type AdminTheme = 'light' | 'dark' | 'system';

interface AdminUIState {
  sidebarCollapsed: boolean;
  sidebarMobileOpen: boolean;
  currentSection: string;
  theme: AdminTheme;

  // Actions
  toggleSidebar: () => void;
  setSidebarCollapsed: (collapsed: boolean) => void;
  toggleMobileSidebar: () => void;
  setCurrentSection: (section: string) => void;
  setTheme: (theme: AdminTheme) => void;
  applyTheme: () => void;
}

function applyThemeToDocument(theme: AdminTheme) {
  if (typeof document === 'undefined') return;
  if (theme === 'system') {
    const prefersDark = window.matchMedia?.('(prefers-color-scheme: dark)').matches;
    document.documentElement.setAttribute('data-theme', prefersDark ? 'dark' : 'light');
  } else {
    document.documentElement.setAttribute('data-theme', theme);
  }
}

export const useAdminStore = create<AdminUIState>()(
  persist(
    (set, get) => ({
      sidebarCollapsed: false,
      sidebarMobileOpen: false,
      currentSection: 'dashboard',
      theme: 'system',

      toggleSidebar: () => set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
      setSidebarCollapsed: (collapsed) => set({ sidebarCollapsed: collapsed }),
      toggleMobileSidebar: () => set((s) => ({ sidebarMobileOpen: !s.sidebarMobileOpen })),
      setCurrentSection: (section) => set({ currentSection: section }),
      setTheme: (theme) => {
        set({ theme });
        applyThemeToDocument(theme);
      },
      applyTheme: () => applyThemeToDocument(get().theme),
    }),
    {
      name: 'ufop-admin-ui',
      storage: createJSONStorage(() => localStorage),
      partialize: (state) => ({
        theme: state.theme,
        sidebarCollapsed: state.sidebarCollapsed,
      }),
    }
  )
);
