'use client';

/**
 * Step-up re-authentication: a sensitive request came back `reauth_required`;
 * `request()` opens the dialog and resolves true once the user confirmed
 * (the API client then retries), false when they cancelled.
 */
import { create } from 'zustand';

export type ReauthMethod = 'totp' | 'passkey' | 'password' | 'sso';

interface ReauthState {
  methods: ReauthMethod[] | null;
  resolve: ((ok: boolean) => void) | null;
  request: (methods: ReauthMethod[]) => Promise<boolean>;
  finish: (ok: boolean) => void;
}

export const useReauthStore = create<ReauthState>((set, get) => ({
  methods: null,
  resolve: null,
  request: (methods) => {
    // One dialog at a time: a second request waits on the same answer.
    const pending = get().resolve;
    return new Promise<boolean>((resolve) => {
      set({
        methods,
        resolve: (ok) => {
          pending?.(ok);
          resolve(ok);
        },
      });
    });
  },
  finish: (ok) => {
    get().resolve?.(ok);
    set({ methods: null, resolve: null });
  },
}));
