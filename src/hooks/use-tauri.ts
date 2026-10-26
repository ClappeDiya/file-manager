/**
 * Tauri IPC availability hook and helper utilities.
 *
 * Provides:
 * - `isTauriAvailable()` - check if running inside Tauri webview
 * - `useTauriAvailable()` - React hook version
 * - `tauriInvoke()` - safe invoke wrapper with fallback
 *
 * When running outside Tauri (e.g., `pnpm dev` in browser),
 * calls fall back to demo/mock data so the UI still works.
 */
import { useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { isServerMode, serverRpc } from "@/lib/server-client";

/** Check if the Tauri IPC bridge is available (i.e., running inside Tauri). */
export function isTauriAvailable(): boolean {
  return typeof window !== "undefined" && ("__TAURI_INTERNALS__" in window || "__TAURI__" in window);
}

/** True inside Tauri or when served by `ufop-server` (commands go over HTTPS). */
export function isBackendAvailable(): boolean {
  return isTauriAvailable() || isServerMode();
}

/** React hook that returns whether Tauri IPC is available. Stable across renders. */
export function useTauriAvailable(): boolean {
  return useMemo(() => isTauriAvailable(), []);
}

// ── App lock: commands wait while the app is locked ──
//
// The backend refuses every command but the lock screen's while the app is
// locked (`app_lock::gate`). Rather than fail all over the interface, a
// call made while locked waits for the unlock and then runs. Identical calls
// (same command and arguments, e.g. a poll) share one wait, so a long lock
// does not pile them up.

/** Mirrors `app_lock::ALLOWED_WHILE_LOCKED`. */
export const ALLOWED_WHILE_LOCKED: ReadonlySet<string> = new Set(["app_lock_status", "app_lock_unlock", "set_ui_language", "get_app_version", "get_portable_status", "greet"]);

let ipcLocked = false;
let unlockWaiters: Array<() => void> = [];
const waitingCalls = new Map<string, Promise<unknown>>();

/** Set by the app lock store when the app locks or unlocks. */
export function setIpcLocked(locked: boolean) {
  ipcLocked = locked;
  if (!locked) {
    const waiters = unlockWaiters;
    unlockWaiters = [];
    waiters.forEach((w) => w());
  }
}

export function isIpcLocked(): boolean {
  return ipcLocked;
}

function whenUnlocked(): Promise<void> {
  return ipcLocked ? new Promise((resolve) => unlockWaiters.push(resolve)) : Promise.resolve();
}

function isLockedError(err: unknown): boolean {
  return !!err && typeof err === "object" && (err as { error_type?: unknown }).error_type === "app_locked";
}

async function invokeRaw<T>(command: string, args: Record<string, unknown> | undefined, viaServer: boolean): Promise<T> {
  if (viaServer || ALLOWED_WHILE_LOCKED.has(command)) {
    return viaServer ? serverRpc<T>(command, args) : invoke<T>(command, args);
  }
  for (;;) {
    if (ipcLocked) {
      const key = `${command}\u0000${JSON.stringify(args ?? null)}`;
      let shared = waitingCalls.get(key) as Promise<T> | undefined;
      if (!shared) {
        shared = whenUnlocked()
          .then(() => invokeRaw<T>(command, args, false))
          .finally(() => waitingCalls.delete(key));
        waitingCalls.set(key, shared);
      }
      return shared;
    }
    try {
      return await invoke<T>(command, args);
    } catch (err) {
      // Locked between our check and the backend's (another window, idle).
      if (!isLockedError(err)) throw err;
      ipcLocked = true;
    }
  }
}

/**
 * Safe Tauri invoke wrapper.
 *
 * - In Tauri: calls `invoke(command, args)` and returns the result.
 * - Outside Tauri: returns the provided `fallback` value (or throws if none given).
 * - On error: logs and returns fallback if provided, otherwise re-throws.
 */
export async function tauriInvoke<T>(
  command: string,
  args?: Record<string, unknown>,
  fallback?: T,
): Promise<T> {
  const viaServer = !isTauriAvailable() && isServerMode();
  if (!isTauriAvailable() && !viaServer) {
    if (fallback !== undefined) {
      return fallback;
    }
    throw new Error(`Tauri IPC not available. Cannot invoke "${command}" outside Tauri.`);
  }

  try {
    return await invokeRaw<T>(command, args, viaServer);
  } catch (err) {
    console.error(`Tauri invoke "${command}" failed:`, err);
    if (fallback !== undefined) {
      return fallback;
    }
    throw err;
  }
}

/**
 * Safe Tauri invoke that never throws - always returns fallback on failure.
 * Useful for non-critical calls where a default is acceptable.
 */
export async function tauriInvokeSafe<T>(
  command: string,
  args: Record<string, unknown> | undefined,
  fallback: T,
): Promise<T> {
  const viaServer = !isTauriAvailable() && isServerMode();
  if (!isTauriAvailable() && !viaServer) {
    return fallback;
  }
  try {
    return await invokeRaw<T>(command, args, viaServer);
  } catch (err) {
    console.error(`Tauri invoke "${command}" failed (using fallback):`, err);
    return fallback;
  }
}
