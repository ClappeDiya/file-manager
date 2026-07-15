/**
 * Notification Bell
 *
 * The render surface for `useUIStore.structuredErrors` — the channel every
 * `reportOperationFailure` / `addStructuredError` call site writes to
 * (`lib/op-error-toast.ts`, ~70 call sites across `file-manager.tsx`,
 * `activity-timeline-panel.tsx`, `lineage-panel.tsx`, `lib/clipboard.ts`,
 * `hooks/use-ledger-tail-poll.ts`).
 *
 * # Why this is a shared component
 *
 * This markup used to live inline in `simple-mode/simple-mode-wrapper.tsx`,
 * which early-returns `<>{fileManager}</>` in Advanced mode. That made the
 * bell — the ONLY consumer of `structuredErrors` — render exclusively in
 * Simple mode, so in Advanced mode every "Delete failed" / "Eject drive
 * failed" accumulated in the store with nothing on screen to show it. The
 * user pressed Delete, the OS refused, and the file silently stayed put.
 *
 * Extracting it lets both shells mount the same surface, the same way
 * `ThemeSwitcher` is already shared between them (see the `appMode !==
 * "simple"` guard in `file-manager.tsx`).
 *
 * # Why it is not a toolbar item
 *
 * `ALL_TOOLBAR_ITEMS` gates optional affordances, but a failure notice is
 * not optional — letting it be toggled off would reintroduce the very
 * silence this component exists to fix. It stays ungated and costs nothing
 * when idle: with no undismissed errors it renders `null`.
 */
import { useState } from "react";
import { Button } from "@ufop/ui-components";
import { Bell } from "lucide-react";
import { useUIStore } from "@/stores/ui-store";

export function NotificationBell() {
  const structuredErrors = useUIStore((s) => s.structuredErrors);
  const dismissError = useUIStore((s) => s.dismissError);
  const [showNotifications, setShowNotifications] = useState(false);

  const undismissedErrors = structuredErrors.filter((e) => !e.dismissed);

  if (undismissedErrors.length === 0) {
    return null;
  }

  return (
    <div className="relative" data-testid="notification-bell">
      <Button
        variant="ghost"
        size="icon"
        onClick={() => setShowNotifications(!showNotifications)}
        aria-label={`${undismissedErrors.length} notification${undismissedErrors.length !== 1 ? "s" : ""}`}
        aria-expanded={showNotifications}
      >
        <Bell className="h-4 w-4" aria-hidden="true" />
        <span className="absolute -top-1 -right-1 h-4 w-4 rounded-full bg-[var(--color-error)] text-[10px] text-white flex items-center justify-center">
          {undismissedErrors.length}
        </span>
      </Button>

      {showNotifications && (
        <div
          className="absolute right-0 top-full mt-2 w-80 max-h-96 overflow-y-auto rounded-lg border border-[var(--color-border)] bg-[var(--color-bg)] shadow-lg z-50"
          data-testid="notification-list"
        >
          <div className="p-3 border-b border-[var(--color-border)]">
            <h3 className="text-sm font-semibold text-[color:var(--color-text)]">
              Notifications
            </h3>
          </div>
          {undismissedErrors.map((error) => (
            <div
              key={error.id}
              className="p-3 border-b border-[var(--color-border)] last:border-b-0"
            >
              <p className="text-sm font-medium text-[color:var(--color-text)]">
                {error.what}
              </p>
              <p className="text-xs text-[color:var(--color-text-secondary)] mt-1">
                {error.why}
              </p>
              {error.userAction && (
                <p className="text-xs text-[color:var(--color-primary)] mt-1">
                  {error.userAction}
                </p>
              )}
              <button
                className="text-xs text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)] mt-1 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-focus-ring)]"
                onClick={() => dismissError(error.id)}
              >
                Dismiss
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
