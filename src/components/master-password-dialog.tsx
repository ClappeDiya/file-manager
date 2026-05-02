/**
 * Master Password Dialog
 *
 * Three modes:
 * - Setup: Create a new master password (password + confirm)
 * - Unlock: Enter master password to unlock the app
 * - Change: Old password + new password + confirm
 *
 * Uses tauriInvoke with isTauriAvailable fallback for dev mode.
 */
import { useState, useCallback, useEffect, useRef } from "react";
import { Lock, Unlock, KeyRound, AlertCircle } from "lucide-react";
import { tauriInvoke, isTauriAvailable } from "@/hooks/use-tauri";
import { useFocusTrap } from "@/hooks/use-focus-trap";

// ── Types ──

export type MasterPasswordMode = "setup" | "unlock" | "change";

export interface MasterPasswordDialogProps {
  /** Which mode to show. */
  mode: MasterPasswordMode;
  /** Called after a successful action (set, unlock, or change). */
  onSuccess: () => void;
  /** Called when the user cancels (only applicable in "change" mode). */
  onCancel?: () => void;
  /** Whether the dialog is visible. */
  open: boolean;
}

// ── Component ──

export function MasterPasswordDialog({
  mode,
  onSuccess,
  onCancel,
  open,
}: MasterPasswordDialogProps) {
  const [password, setPassword] = useState("");
  const [confirmPassword, setConfirmPassword] = useState("");
  const [oldPassword, setOldPassword] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  // Trap focus inside the dialog and restore on close. Escape only closes
  // when a cancel handler is provided (Setup/Unlock paths must complete).
  const containerRef = useFocusTrap<HTMLDivElement>({
    active: open,
    onEscape: onCancel,
    initialFocusRef: inputRef,
  });

  useEffect(() => {
    if (open) {
      setPassword("");
      setConfirmPassword("");
      setOldPassword("");
      setError("");
    }
  }, [open, mode]);

  const handleSubmit = useCallback(
    async (e: React.FormEvent) => {
      e.preventDefault();
      setError("");

      if (!isTauriAvailable()) {
        // Dev mode stub
        onSuccess();
        return;
      }

      try {
        setLoading(true);

        if (mode === "setup") {
          if (password.length < 8) {
            setError("Password must be at least 8 characters.");
            return;
          }
          if (password !== confirmPassword) {
            setError("Passwords do not match.");
            return;
          }
          await tauriInvoke("set_master_password", { password });
          onSuccess();
        } else if (mode === "unlock") {
          if (!password) {
            setError("Please enter your master password.");
            return;
          }
          const ok = await tauriInvoke<boolean>("master_unlock_app", {
            password,
          });
          if (ok) {
            onSuccess();
          } else {
            setError("Incorrect password. Please try again.");
          }
        } else if (mode === "change") {
          if (!oldPassword) {
            setError("Please enter your current password.");
            return;
          }
          if (password.length < 8) {
            setError("New password must be at least 8 characters.");
            return;
          }
          if (password !== confirmPassword) {
            setError("New passwords do not match.");
            return;
          }
          await tauriInvoke("change_master_password", {
            oldPassword,
            newPassword: password,
          });
          onSuccess();
        }
      } catch (err: unknown) {
        const msg =
          err instanceof Error
            ? err.message
            : typeof err === "string"
              ? err
              : "An unexpected error occurred.";
        setError(msg);
      } finally {
        setLoading(false);
      }
    },
    [mode, password, confirmPassword, oldPassword, onSuccess],
  );

  if (!open) return null;

  const title =
    mode === "setup"
      ? "Create Master Password"
      : mode === "unlock"
        ? "Unlock App"
        : "Change Master Password";

  const submitLabel =
    mode === "setup"
      ? "Set Password"
      : mode === "unlock"
        ? "Unlock"
        : "Change Password";

  const IconComponent = mode === "unlock" ? Lock : KeyRound;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-sm" role="dialog" aria-modal="true" aria-labelledby="master-password-title">
      <div ref={containerRef} className="w-full max-w-sm mx-4 rounded-lg border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-900 shadow-xl">
        {/* Header */}
        <div className="flex items-center gap-2 px-4 py-3 border-b border-gray-200 dark:border-gray-700">
          <IconComponent className="w-4 h-4 text-blue-500" aria-hidden="true" />
          <h2 id="master-password-title" className="text-sm font-semibold">{title}</h2>
        </div>

        {/* Form */}
        <form onSubmit={handleSubmit} className="p-4 space-y-3">
          {mode === "setup" && (
            <p className="text-xs text-gray-500 dark:text-gray-400">
              This password will protect your saved connection credentials. Make
              sure to remember it — it cannot be recovered.
            </p>
          )}

          {/* Old password (change mode only) */}
          {mode === "change" && (
            <div>
              <label className="block text-xs font-medium mb-1">
                Current Password
              </label>
              <input
                ref={inputRef}
                type="password"
                value={oldPassword}
                onChange={(e) => setOldPassword(e.target.value)}
                required
                autoComplete="current-password"
                className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 focus:outline-none focus:ring-1 focus:ring-blue-500"
              />
            </div>
          )}

          {/* Main password field */}
          <div>
            <label className="block text-xs font-medium mb-1">
              {mode === "change" ? "New Password" : "Master Password"}
            </label>
            <input
              ref={mode !== "change" ? inputRef : undefined}
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              required
              minLength={mode === "unlock" ? 1 : 8}
              autoComplete={
                mode === "unlock" ? "current-password" : "new-password"
              }
              placeholder={mode === "unlock" ? "" : "Min 8 characters"}
              className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 focus:outline-none focus:ring-1 focus:ring-blue-500"
            />
          </div>

          {/* Confirm password (setup and change modes) */}
          {(mode === "setup" || mode === "change") && (
            <div>
              <label className="block text-xs font-medium mb-1">
                Confirm {mode === "change" ? "New " : ""}Password
              </label>
              <input
                type="password"
                value={confirmPassword}
                onChange={(e) => setConfirmPassword(e.target.value)}
                required
                minLength={8}
                autoComplete="new-password"
                className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 focus:outline-none focus:ring-1 focus:ring-blue-500"
              />
            </div>
          )}

          {/* Error message */}
          {error && (
            <div className="flex items-start gap-1.5 text-xs text-red-500">
              <AlertCircle className="w-3.5 h-3.5 mt-0.5 flex-shrink-0" />
              <span>{error}</span>
            </div>
          )}

          {/* Actions */}
          <div className="flex justify-end gap-2 pt-1">
            {(mode === "change" || onCancel) && (
              <button
                type="button"
                onClick={onCancel}
                disabled={loading}
                className="text-sm px-3 py-1.5 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800 disabled:opacity-50"
              >
                Cancel
              </button>
            )}
            <button
              type="submit"
              disabled={loading}
              className="text-sm px-3 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50 flex items-center gap-1.5"
            >
              {loading ? (
                "Working..."
              ) : (
                <>
                  {mode === "unlock" ? (
                    <Unlock className="w-3.5 h-3.5" />
                  ) : (
                    <KeyRound className="w-3.5 h-3.5" />
                  )}
                  {submitLabel}
                </>
              )}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

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
