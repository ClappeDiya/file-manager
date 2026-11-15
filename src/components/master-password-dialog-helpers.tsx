import { useState, useCallback, useEffect } from "react";
import { tauriInvoke, isTauriAvailable } from "@/hooks/use-tauri";


// ── Hook for master password state ──

export interface MasterPasswordState {
  /** Whether a master password has been configured. */
  isSet: boolean;
  /** Whether the app is currently unlocked. */
  isUnlocked: boolean;
  /** Re-check the master password state from the backend. */
  refresh: () => Promise<void>;
  /** Loading state while checking. */
  checking: boolean;
}


/**
 * Hook to query master password status from the backend.
 * Returns whether the password is set and whether the app is unlocked.
 */
export function useMasterPasswordState(): MasterPasswordState {
  const [isSet, setIsSet] = useState(false);
  const [isUnlocked, setIsUnlocked] = useState(false);
  const [checking, setChecking] = useState(true);

  const refresh = useCallback(async () => {
    if (!isTauriAvailable()) {
      // Dev mode: assume no password set, unlocked
      setIsSet(false);
      setIsUnlocked(true);
      setChecking(false);
      return;
    }

    try {
      setChecking(true);
      const [set, unlocked] = await Promise.all([
        tauriInvoke<boolean>("is_master_password_set", undefined, false),
        tauriInvoke<boolean>("is_app_unlocked", undefined, false),
      ]);
      setIsSet(set);
      setIsUnlocked(unlocked);
    } catch {
      // If we can't reach the backend, assume defaults
      setIsSet(false);
      setIsUnlocked(true);
    } finally {
      setChecking(false);
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  return { isSet, isUnlocked, refresh, checking };
}


/**
 * Helper to lock the app via IPC.
 */
export async function lockApp(): Promise<void> {
  if (!isTauriAvailable()) return;
  await tauriInvoke("master_lock_app");
}
