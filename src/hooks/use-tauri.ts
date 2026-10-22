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
    return viaServer ? await serverRpc<T>(command, args) : await invoke<T>(command, args);
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
    return viaServer ? await serverRpc<T>(command, args) : await invoke<T>(command, args);
  } catch (err) {
    console.error(`Tauri invoke "${command}" failed (using fallback):`, err);
    return fallback;
  }
}
