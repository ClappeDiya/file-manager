/**
 * Activity Feed Component (T-057)
 *
 * Shows the last 100 operations with:
 * - Plain language descriptions
 * - Undo buttons where applicable
 * - Structured error display (what/why/did/next pattern)
 * - Timestamp and type indicators
 *
 * Part of the content design system ensuring every message
 * is clear, actionable, and uses no unexplained acronyms.
 */
import { useCallback } from "react";
import { useUIStore, type ActivityEntry, type ActivityType } from "@/stores/ui-store";
import { useFileManagerStore } from "@/stores/file-manager-store";
import { cn } from "@ufop/ui-components";
import { formatRelativeTime } from "@/lib/ledger-dispatch";
import { Button } from "@ufop/ui-components";
import { ScrollArea } from "@ufop/ui-components";
import {
  Copy,
  ArrowRight,
  Pencil,
  Trash2,
  FolderPlus,
  FilePlus,
  RefreshCw,
  ArrowUpDown,
  Plug,
  Unplug,
  AlertTriangle,
  Undo2,
  Clock,
  X,
} from "lucide-react";

// ──────────────────────────────────────────────
// Activity type metadata
// ──────────────────────────────────────────────

function getActivityIcon(type: ActivityType): React.ReactNode {
  const iconClass = "h-4 w-4";
  switch (type) {
    case "copy":
      return <Copy className={iconClass} aria-hidden="true" />;
    case "move":
      return <ArrowRight className={iconClass} aria-hidden="true" />;
    case "rename":
      return <Pencil className={iconClass} aria-hidden="true" />;
    case "delete":
      return <Trash2 className={iconClass} aria-hidden="true" />;
    case "create_folder":
      return <FolderPlus className={iconClass} aria-hidden="true" />;
    case "create_file":
      return <FilePlus className={iconClass} aria-hidden="true" />;
    case "sync":
      return <RefreshCw className={iconClass} aria-hidden="true" />;
    case "transfer":
      return <ArrowUpDown className={iconClass} aria-hidden="true" />;
    case "connect":
      return <Plug className={iconClass} aria-hidden="true" />;
    case "disconnect":
      return <Unplug className={iconClass} aria-hidden="true" />;
    case "error":
      return <AlertTriangle className={iconClass} aria-hidden="true" />;
    case "undo":
      return <Undo2 className={iconClass} aria-hidden="true" />;
    default:
      return <Clock className={iconClass} aria-hidden="true" />;
  }
}

function getActivityColor(type: ActivityType): string {
  switch (type) {
    case "error":
      return "text-[color:var(--color-error)] bg-[var(--color-error)]/10";
    case "delete":
      return "text-[color:var(--color-warning)] bg-[var(--color-warning)]/10";
    case "copy":
    case "move":
    case "transfer":
      return "text-[color:var(--color-primary)] bg-[var(--color-primary)]/10";
    case "sync":
      return "text-[color:var(--color-success,#22c55e)] bg-[var(--color-success,#22c55e)]/10";
    case "undo":
      return "text-[color:var(--color-text-secondary)] bg-[var(--color-bg-secondary)]";
    default:
      return "text-[color:var(--color-text-secondary)] bg-[var(--color-bg-secondary)]";
  }
}


// ──────────────────────────────────────────────
// Activity Feed Component
// ──────────────────────────────────────────────

export function ActivityFeed() {
  const activityFeed = useUIStore((s) => s.activityFeed);
  const clearActivity = useUIStore((s) => s.clearActivity);
  const addActivity = useUIStore((s) => s.addActivity);
  const popUndo = useFileManagerStore((s) => s.popUndo);

  const handleUndo = useCallback(
    (entry: ActivityEntry) => {
      const undoEntry = popUndo();
      if (undoEntry) {
        addActivity({
          type: "undo",
          summary: `Undid: ${entry.summary}`,
          paths: entry.paths,
          undoable: false,
        });
      }
    },
    [popUndo, addActivity],
  );

  return (
    <div className="flex flex-col h-full" data-testid="activity-feed">
      {/* Header */}
      <div className="flex items-center justify-between px-6 py-4 border-b border-[var(--color-border)]">
        <div>
          <h2 className="text-lg font-semibold text-[color:var(--color-text)]">
            Activity
          </h2>
          <p className="text-xs text-[color:var(--color-text-secondary)] mt-0.5">
            Your recent file operations
          </p>
        </div>
        {activityFeed.length > 0 && (
          <Button
            variant="ghost"
            size="sm"
            onClick={clearActivity}
            aria-label="Clear all activity"
          >
            <X className="h-3.5 w-3.5 mr-1" aria-hidden="true" />
            Clear
          </Button>
        )}
      </div>

      {/* Activity list */}
      <ScrollArea className="flex-1">
        {activityFeed.length === 0 ? (
          <div className="flex flex-col items-center justify-center h-64 text-center px-6">
            <Clock className="h-10 w-10 text-[color:var(--color-text-tertiary)] mb-3" aria-hidden="true" />
            <p className="text-sm text-[color:var(--color-text-secondary)]">
              No recent activity
            </p>
            <p className="text-xs text-[color:var(--color-text-tertiary)] mt-1">
              Actions like copying, moving, renaming, and syncing files will
              appear here.
            </p>
          </div>
        ) : (
          <div className="divide-y divide-[var(--color-border)]">
            {activityFeed.map((entry) => (
              <ActivityRow
                key={entry.id}
                entry={entry}
                onUndo={entry.undoable ? () => handleUndo(entry) : undefined}
              />
            ))}
          </div>
        )}
      </ScrollArea>

      {/* Footer summary */}
      {activityFeed.length > 0 && (
        <div className="px-6 py-2 border-t border-[var(--color-border)] text-xs text-[color:var(--color-text-tertiary)]">
          Showing {activityFeed.length} of last 100 operations
        </div>
      )}
    </div>
  );
}

// ──────────────────────────────────────────────
// Activity Row
// ──────────────────────────────────────────────

function ActivityRow({
  entry,
  onUndo,
}: {
  entry: ActivityEntry;
  onUndo?: () => void;
}) {
  return (
    <div
      className="px-6 py-3 hover:bg-[var(--color-hover-bg)] transition-colors"
      role="listitem"
    >
      <div className="flex items-start gap-3">
        {/* Icon */}
        <div
          className={cn(
            "h-8 w-8 rounded-lg flex items-center justify-center shrink-0 mt-0.5",
            getActivityColor(entry.type),
          )}
        >
          {getActivityIcon(entry.type)}
        </div>

        {/* Content */}
        <div className="flex-1 min-w-0">
          <p className="text-sm text-[color:var(--color-text)]">
            {entry.summary}
          </p>

          {/* App action explanation */}
          {entry.appAction && (
            <p className="text-xs text-[color:var(--color-text-secondary)] mt-1">
              {entry.appAction}
            </p>
          )}

          {/* Next step guidance */}
          {entry.nextStep && (
            <p className="text-xs text-[color:var(--color-primary)] mt-1">
              {entry.nextStep}
            </p>
          )}

          {/* Affected paths (collapsed if many) */}
          {entry.paths.length > 0 && entry.paths.length <= 3 && (
            <div className="mt-1.5 space-y-0.5">
              {entry.paths.map((path, i) => (
                <p
                  key={i}
                  className="text-xs text-[color:var(--color-text-tertiary)] truncate font-mono"
                  title={path}
                >
                  {path}
                </p>
              ))}
            </div>
          )}
          {entry.paths.length > 3 && (
            <p className="text-xs text-[color:var(--color-text-tertiary)] mt-1.5">
              {entry.paths.length} files affected
            </p>
          )}

          {/* Timestamp */}
          <p className="text-xs text-[color:var(--color-text-tertiary)] mt-1.5">
            {formatRelativeTime(entry.timestamp, undefined, "prose")}
          </p>
        </div>

        {/* Undo button */}
        {onUndo && (
          <Button
            variant="ghost"
            size="sm"
            onClick={onUndo}
            className="shrink-0"
            aria-label={`Undo: ${entry.summary}`}
            title="Undo this action"
          >
            <Undo2 className="h-3.5 w-3.5" aria-hidden="true" />
          </Button>
        )}
      </div>
    </div>
  );
}

// ──────────────────────────────────────────────
// Structured Error Banner (T-057)
// ──────────────────────────────────────────────
// Reusable component that displays errors following the
// what/why/did/next content design pattern.

export function StructuredErrorBanner({
  what,
  why,
  appDid,
  userAction,
  onDismiss,
}: {
  what: string;
  why: string;
  appDid: string;
  userAction: string;
  onDismiss?: () => void;
}) {
  return (
    <div
      className="p-4 rounded-lg border border-[var(--color-error)]/30 bg-[var(--color-error)]/5"
      role="alert"
    >
      <div className="flex items-start gap-3">
        <AlertTriangle className="h-5 w-5 text-[color:var(--color-error)] shrink-0 mt-0.5" aria-hidden="true" />
        <div className="flex-1 min-w-0">
          {/* What happened */}
          <p className="text-sm font-medium text-[color:var(--color-text)]">
            {what}
          </p>

          {/* Why it happened */}
          <p className="text-xs text-[color:var(--color-text-secondary)] mt-1">
            <span className="font-medium">Why:</span> {why}
          </p>

          {/* What the app did */}
          <p className="text-xs text-[color:var(--color-text-secondary)] mt-1">
            <span className="font-medium">What we did:</span> {appDid}
          </p>

          {/* What the user can do */}
          <p className="text-xs text-[color:var(--color-primary)] mt-1.5 font-medium">
            {userAction}
          </p>
        </div>

        {onDismiss && (
          <button
            onClick={onDismiss}
            className="text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)] shrink-0"
            aria-label="Dismiss error"
          >
            <X className="h-4 w-4" aria-hidden="true" />
          </button>
        )}
      </div>
    </div>
  );
}

// ──────────────────────────────────────────────
// Destructive Operation Confirmation (T-057)
// ──────────────────────────────────────────────
// Shows clear consequences before destructive operations.

export function DestructiveConfirmation({
  title,
  description,
  consequences,
  recoveryInfo,
  onConfirm,
  onCancel,
  confirmLabel,
}: {
  title: string;
  description: string;
  consequences: string[];
  recoveryInfo: string;
  onConfirm: () => void;
  onCancel: () => void;
  confirmLabel?: string;
}) {
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-sm"
      role="alertdialog"
      aria-modal="true"
      aria-label={title}
    >
      <div className="w-full max-w-md mx-4 p-6 bg-[var(--color-bg)] rounded-xl shadow-2xl">
        <div className="flex items-start gap-3 mb-4">
          <AlertTriangle className="h-6 w-6 text-[color:var(--color-warning)] shrink-0" aria-hidden="true" />
          <div>
            <h3 className="text-base font-semibold text-[color:var(--color-text)]">
              {title}
            </h3>
            <p className="text-sm text-[color:var(--color-text-secondary)] mt-1">
              {description}
            </p>
          </div>
        </div>

        {/* Consequences */}
        <div className="mb-4 p-3 rounded-lg bg-[var(--color-warning)]/5 border border-[var(--color-warning)]/20">
          <p className="text-xs font-medium text-[color:var(--color-text)] mb-2">
            This will:
          </p>
          <ul className="space-y-1">
            {consequences.map((c, i) => (
              <li key={i} className="text-xs text-[color:var(--color-text-secondary)] flex items-start gap-2">
                <span className="text-[color:var(--color-warning)] mt-0.5">*</span>
                {c}
              </li>
            ))}
          </ul>
        </div>

        {/* Recovery info */}
        <p className="text-xs text-[color:var(--color-text-secondary)] mb-5">
          <span className="font-medium">Recovery:</span> {recoveryInfo}
        </p>

        {/* Actions */}
        <div className="flex gap-3 justify-end">
          <Button variant="ghost" onClick={onCancel}>
            Cancel
          </Button>
          <Button
            variant="destructive"
            onClick={onConfirm}
          >
            {confirmLabel || "Delete"}
          </Button>
        </div>
      </div>
    </div>
  );
}
