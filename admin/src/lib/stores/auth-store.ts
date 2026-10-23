'use client';

import { create } from 'zustand';
import { api } from '@/lib/api-client';
import { persist } from 'zustand/middleware';
import type { AdminUser, Permission, Role } from '../types/auth';
import { hasPermission } from '../utils/permissions';
import { setDisplayTimeZone } from '../utils/format';
import { followAccountLocale } from '../i18n';

export interface OrgSummary {
  id: string;
  name: string;
  path: string;
  role: Role;
  parentId: string | null;
}

/** Outcome of the password step: signed in, or a second factor is needed. */
export type LoginOutcome =
  | { ok: true; redirect: string }
  | { ok: false; mfaChallengeId: string; redirect: string }
  | { ok: false; passwordChangeToken: string }
  | { ok: false; error: string };

interface AuthState {
  user: AdminUser | null;
  /** Permissions in the current organization, computed by the server. */
  permissions: Permission[] | null;
  organization: { id: string; name: string; path: string; status: string } | null;
  organizations: OrgSummary[];
  /** Read-only "view as": the administrator really signed in, and until when. */
  viewAs: { actor: { email: string; name: string }; until: string } | null;
  isAuthenticated: boolean;
  isLoading: boolean;
  isValidating: boolean;
  error: string | null;

  // Actions
  setUser: (user: AdminUser | null) => void;
  login: (email: string, password: string, redirect?: string) => Promise<LoginOutcome>;
  verifyMfa: (challengeId: string, code: string, redirect?: string) => Promise<LoginOutcome>;
  switchOrg: (orgId: string) => Promise<void>;
  logout: () => void;
  checkPermission: (permission: Permission) => boolean;
  setError: (error: string | null) => void;
  validateSession: () => Promise<void>;
  startViewAs: (userId: string) => Promise<void>;
  stopViewAs: () => Promise<void>;
}

export const useAuthStore = create<AuthState>()(
  persist(
    (set, get) => ({
      user: null,
      permissions: null,
      organization: null,
      organizations: [],
      viewAs: null,
      isAuthenticated: false,
      isLoading: false,
      isValidating: false,
      error: null,

      setUser: (user) => {
        setDisplayTimeZone(user?.timeZone);
        followAccountLocale(user?.locale);
        set({ user, isAuthenticated: !!user, error: null });
      },

      login: async (email: string, password: string, redirect?: string) => {
        set({ isLoading: true, error: null });
        try {
          const response = await fetch('/api/auth/login', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ email, password, redirect }),
          });
          const data = await response.json();
          if (!response.ok && data.code === 'password_expired' && data.changeToken) {
            set({ isLoading: false });
            return { ok: false, passwordChangeToken: data.changeToken as string };
          }
          if (!response.ok) throw new Error(data.error || 'Login failed');
          set({ isLoading: false });
          if (data.mfaRequired) return { ok: false, mfaChallengeId: data.challengeId, redirect: data.redirect ?? '/' };
          await get().validateSession();
          return { ok: true, redirect: data.redirect ?? '/' };
        } catch (error) {
          const message = error instanceof Error ? error.message : 'Login failed';
          set({ isLoading: false, error: message });
          return { ok: false, error: message };
        }
      },

      verifyMfa: async (challengeId: string, code: string, redirect?: string) => {
        set({ isLoading: true, error: null });
        try {
          const response = await fetch('/api/auth/mfa/verify', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ challengeId, code, redirect }),
          });
          const data = await response.json();
          if (!response.ok) throw new Error(data.error || 'Verification failed');
          set({ isLoading: false });
          await get().validateSession();
          return { ok: true, redirect: data.redirect ?? '/' };
        } catch (error) {
          const message = error instanceof Error ? error.message : 'Verification failed';
          set({ isLoading: false, error: message });
          return { ok: false, error: message };
        }
      },

      switchOrg: async (orgId: string) => {
        const response = await fetch('/api/orgs/switch', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ orgId }) });
        if (!response.ok) {
          const data = await response.json().catch(() => ({}));
          throw new Error(data.error || 'Could not switch organization');
        }
        await get().validateSession();
      },

      logout: () => {
        set({ user: null, permissions: null, organization: null, organizations: [], isAuthenticated: false, error: null });
        // Clear the server-side session cookie
        fetch('/api/auth/logout', { method: 'POST' }).catch(() => {});
      },

      checkPermission: (permission: Permission) => {
        const { user, permissions } = get();
        if (!user) return false;
        return permissions ? permissions.includes(permission) : hasPermission(user.role, permission);
      },

      setError: (error) => set({ error }),

      startViewAs: async (userId) => {
        await api('/api/view-as', { method: 'POST', body: { userId } });
        await get().validateSession();
      },

      stopViewAs: async () => {
        await api('/api/view-as', { method: 'DELETE' });
        await get().validateSession();
      },

      validateSession: async () => {
        set({ isValidating: true });
        try {
          const response = await fetch('/api/auth/session');
          if (!response.ok) {
            set({ user: null, permissions: null, isAuthenticated: false, isValidating: false });
            return;
          }
          const { user, permissions, organization, organizations, viewAs } = await response.json();
          setDisplayTimeZone(user?.timeZone);
          followAccountLocale(user?.locale);
          set({ user, permissions: permissions ?? null, organization: organization ?? null, organizations: organizations ?? [], viewAs: viewAs ?? null, isAuthenticated: true, isValidating: false });
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
