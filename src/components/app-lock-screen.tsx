/**
 * The lock screen (Settings → App Lock). `AppLockGate` wraps the window:
 * while the app is locked the content underneath is inert and hidden from
 * assistive technology, and its commands wait (`use-tauri.ts`). It also
 * reports use of the window so the idle timer (in the backend, shared by
 * all windows) restarts.
 */
import { useEffect, useRef, useState, type ReactNode } from "react";
import { Lock } from "lucide-react";
import { isTauriAvailable, tauriInvokeSafe } from "@/hooks/use-tauri";
import { errorText } from "@/lib/error-text";
import { LOCK_EVENT, useAppLockStore } from "@/stores/app-lock-store";

/** Reports of use are sent at most this often (ms). */
export const ACTIVITY_INTERVAL_MS = 20_000;

export function LockScreen() {
  const unlock = useAppLockStore((s) => s.unlock);
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => input.current?.focus(), []);

  const submit = async () => {
    if (!password || busy) return;
    setBusy(true);
    setError(null);
    try {
      if (!(await unlock(password))) setError("Wrong password.");
    } catch (e) {
      // e.g. too many wrong attempts: the password manager waits a while.
      setError(errorText(e));
    } finally {
      setBusy(false);
      setPassword("");
      input.current?.focus();
    }
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="app-lock-title"
      className="fixed inset-0 z-[2147483647] flex items-center justify-center bg-[var(--color-bg)]"
      data-testid="app-lock-screen"
    >
      <form
        className="w-72 space-y-3 text-center"
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <Lock className="mx-auto h-10 w-10 text-[color:var(--color-primary)]" aria-hidden="true" />
        <h1 id="app-lock-title" className="text-lg font-semibold text-[color:var(--color-text)]">
          UFOP is locked
        </h1>
        <p className="text-xs text-[color:var(--color-text-secondary)]">Enter your master password. Scheduled work keeps running while the app is locked.</p>
        <input
          ref={input}
          type="password"
          autoComplete="current-password"
          aria-label="Master password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          className="w-full px-3 py-2 rounded border border-[var(--color-border)] bg-transparent text-sm"
          data-testid="app-lock-password"
        />
        {error && (
          <p role="alert" className="text-xs text-red-600" data-testid="app-lock-error">
            {error}
          </p>
        )}
        <button
          type="submit"
          disabled={!password || busy}
          className="w-full px-3 py-2 rounded bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)] text-sm disabled:opacity-50"
          data-testid="app-lock-unlock"
        >
          {busy ? "Checking…" : "Unlock"}
        </button>
      </form>
    </div>
  );
}

export function AppLockGate({ children }: { children: ReactNode }) {
  const status = useAppLockStore((s) => s.status);
  const locked = !!status?.locked;
  // Until the state is known (lock at start), show nothing of the app.
  const pending = isTauriAvailable() && status === null;

  // Follow what the backend (or another window) does, then learn the state:
  // listening first, so a lock in between is not missed.
  useEffect(() => {
    if (!isTauriAvailable()) return;
    let off: (() => void) | undefined;
    let live = true;
    const unlockedDefault = { enabled: false, locked: false, lock_on_start: false, idle_minutes: 0, has_password: false };
    const load = () =>
      useAppLockStore
        .getState()
        .load()
        .catch(() => {
          // No answer: never leave the window hidden behind an unknown state.
          if (!useAppLockStore.getState().status) useAppLockStore.setState({ status: unlockedDefault });
        });
    void import("@tauri-apps/api/event")
      .then(({ listen }) => listen<{ locked: boolean }>(LOCK_EVENT, (e) => useAppLockStore.getState().setLocked(!!e.payload?.locked)))
      .then((un) => {
        if (live) off = un;
        else un();
      })
      .catch(() => {})
      .finally(() => void load());
    return () => {
      live = false;
      off?.();
    };
  }, []);

  // Use of this window restarts the idle timer.
  const watchIdle = !!status?.enabled && !locked && status.idle_minutes > 0;
  useEffect(() => {
    if (!watchIdle) return;
    let last = 0;
    const onUse = () => {
      const now = Date.now();
      if (now - last < ACTIVITY_INTERVAL_MS) return;
      last = now;
      void tauriInvokeSafe("app_lock_activity", undefined, null);
    };
    const events = ["pointerdown", "keydown", "wheel", "pointermove"] as const;
    events.forEach((ev) => window.addEventListener(ev, onUse, { passive: true }));
    return () => events.forEach((ev) => window.removeEventListener(ev, onUse));
  }, [watchIdle]);

  return (
    <>
      <div className={pending ? "h-full invisible" : "h-full"} inert={locked || pending} aria-hidden={locked || pending || undefined} data-testid="app-lock-content">
        {children}
      </div>
      {locked && <LockScreen />}
    </>
  );
}
