/**
 * App lock state for this window (see `src-tauri/src/app_lock.rs`). The
 * backend owns the lock; every window learns about changes from the
 * `ufop://app-lock` event and pauses its commands while locked
 * (`setIpcLocked`).
 */
import { create } from "zustand";
import { isTauriAvailable, setIpcLocked, setLockRefusalListener, tauriInvoke } from "@/hooks/use-tauri";

/** Mirrors `app_lock::AppLockStatus`. */
export interface AppLockStatus {
  enabled: boolean;
  locked: boolean;
  lock_on_start: boolean;
  idle_minutes: number;
  has_password: boolean;
}

/** Mirrors `app_lock::AppLockSettings`. */
export interface AppLockSettings {
  enabled: boolean;
  lock_on_start: boolean;
  idle_minutes: number;
}

/** Mirrors `app_lock::IDLE_CHOICES`. */
export const IDLE_CHOICES = [0, 1, 5, 10, 15, 30, 60] as const;

export const LOCK_EVENT = "ufop://app-lock";

interface AppLockState {
  status: AppLockStatus | null;
  /** Ask the backend (at start, and when an event arrives). */
  load: () => Promise<void>;
  /** `true` when the password was right. */
  unlock: (password: string) => Promise<boolean>;
  lockNow: () => Promise<void>;
  save: (settings: AppLockSettings, password?: string | null, newPassword?: string | null) => Promise<void>;
  /** The backend said the app locked or unlocked. */
  setLocked: (locked: boolean) => void;
}

function apply(status: AppLockStatus) {
  setIpcLocked(status.locked);
  return { status };
}

export const useAppLockStore = create<AppLockState>()((set, get) => ({
  status: null,
  load: async () => {
    if (!isTauriAvailable()) return;
    set(apply(await tauriInvoke<AppLockStatus>("app_lock_status")));
  },
  unlock: async (password) => {
    const ok = await tauriInvoke<boolean>("app_lock_unlock", { password });
    if (ok) await get().load();
    return ok;
  },
  lockNow: async () => {
    await tauriInvoke("app_lock_now");
    get().setLocked(true);
  },
  save: async (settings, password = null, newPassword = null) => {
    set(apply(await tauriInvoke<AppLockStatus>("app_lock_set_settings", { settings, password, newPassword })));
  },
  setLocked: (locked) => {
    const cur = get().status;
    setIpcLocked(locked);
    if (cur) set({ status: { ...cur, locked } });
    else void get().load();
  },
}));

// A call the backend refused as locked: learn the real state (the lock
// screen shows when it is locked; a stale refusal just lets the calls go on).
setLockRefusalListener(() => void useAppLockStore.getState().load().catch(() => {}));
