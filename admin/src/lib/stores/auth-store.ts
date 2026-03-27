'use client';

import { create } from 'zustand';
import { persist } from 'zustand/middleware';
import type { AdminUser, Role, Permission } from '../types/auth';
import { hasPermission, canAccessAdmin } from '../utils/permissions';

interface AuthState {
  user: AdminUser | null;
  isAuthenticated: boolean;
  isLoading: boolean;
  isValidating: boolean;
  error: string | null;

  // Actions
  setUser: (user: AdminUser | null) => void;
  login: (email: string, password: string) => Promise<void>;
  logout: () => void;
  checkPermission: (permission: Permission) => boolean;
  setError: (error: string | null) => void;
  validateSession: () => Promise<void>;
}

export const useAuthStore = create<AuthState>()(
  persist(
    (set, get) => ({
      user: null,
      isAuthenticated: false,
      isLoading: false,
      isValidating: false,
      error: null,

      setUser: (user) => {
        set({ user, isAuthenticated: !!user, error: null });
      },

      login: async (email: string, password: string) => {
        set({ isLoading: true, error: null });
        try {
          const response = await fetch('/api/auth/login', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ email, password }),
          });

          if (!response.ok) {
            const data = await response.json();
            throw new Error(data.error || 'Login failed');
          }

          const { user } = await response.json();

          if (!canAccessAdmin(user.role)) {
            throw new Error('Insufficient permissions to access admin console');
          }

          set({ user, isAuthenticated: true, isLoading: false });
        } catch (error) {
          set({
            isLoading: false,
            error: error instanceof Error ? error.message : 'Login failed',
          });
        }
      },

      logout: () => {
        set({ user: null, isAuthenticated: false, error: null });
        // Clear the server-side session cookie
        fetch('/api/auth/logout', { method: 'POST' }).catch(() => {});
      },

      checkPermission: (permission: Permission) => {
        const { user } = get();
        if (!user) return false;
        return hasPermission(user.role, permission);
      },

      setError: (error) => set({ error }),

      validateSession: async () => {
        set({ isValidating: true });
        try {
          const response = await fetch('/api/auth/session');
          if (!response.ok) {
            set({ user: null, isAuthenticated: false, isValidating: false });
            return;
          }
          const { user } = await response.json();
          set({ user, isAuthenticated: true, isValidating: false });
        } catch {
          set({ user: null, isAuthenticated: false, isValidating: false });
        }
      },
    }),
    {
      name: 'ufop-admin-auth',
      partialize: () => ({}),
    }
  )
);
