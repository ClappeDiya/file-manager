/**
 * SinceLastSeenToast — zero-effort "what happened while you were away?"
 *
 * Renders a single non-intrusive dismissible card when the backend
 * ledger reports activity since the user's last session. Auto-hides
 * after 12s so it never blocks interaction. Renders nothing when the
 * summary total is 0 (fresh install or no background activity).
 *
 * This is the user-visible payoff of Phases 1-4 of the Operation
 * Ledger: fs + transfer + sync + automation lifecycle events are
 * summarised into a single glance.
 */

import { useEffect, useState } from "react";

/** Backend payload — must match Rust `LedgerSinceSummary` exactly. */
export interface LedgerSinceSummary {
  since: string;
  total: number;
  by_engine: Record<string, number>;
  by_status: Record<string, number>;
}

interface SinceLastSeenToastProps {
  summary: LedgerSinceSummary | null;
  onDismiss: () => void;
}

// Human-readable labels for engines. Centralised so future engines just
// need one new entry.
const ENGINE_LABELS: Record<string, string> = {
  fs: "file operation",
  transfer: "transfer",
  sync: "sync",
  automation: "automation",
  mount: "mount",
  spaces: "space",
  ai: "AI action",
  vault: "vault event",
  compat: "compatibility fix",
  connector: "connector event",
  system: "system event",
};

/** Pluralise the labels when count != 1. Cheap, local, no i18n dep. */
function pluralise(label: string, count: number): string {
  if (count === 1) return label;
  // "file operation" -> "file operations", "sync" -> "syncs"
  return `${label}s`;
}

/**
 * Turn `{ automation: 3, sync: 1, fs: 8 }` into
 * `"3 automations · 1 sync · 8 file operations"`.
 * Stable BTreeMap ordering from Rust means no re-shuffling between runs.
 */
function formatBreakdown(byEngine: Record<string, number>): string {
  const parts: string[] = [];
  for (const [engine, count] of Object.entries(byEngine)) {
    if (count <= 0) continue;
    const label = ENGINE_LABELS[engine] ?? engine;
    parts.push(`${count} ${pluralise(label, count)}`);
  }
  return parts.join(" · ");
}

export function SinceLastSeenToast({ summary, onDismiss }: SinceLastSeenToastProps) {
  const [visible, setVisible] = useState(true);

  // Auto-dismiss after 12 seconds so the toast never lingers.
  useEffect(() => {
    if (!summary || summary.total <= 0) return;
    const timer = setTimeout(() => {
      setVisible(false);
      onDismiss();
    }, 12_000);
    return () => clearTimeout(timer);
  }, [summary, onDismiss]);

  if (!summary || summary.total <= 0 || !visible) return null;

  const breakdown = formatBreakdown(summary.by_engine);
  const failed = summary.by_status.failed ?? 0;

  return (
    <div
      role="status"
      aria-live="polite"
      style={{
        position: "fixed",
        bottom: 16,
        right: 16,
        zIndex: 40,
        maxWidth: 360,
        padding: "12px 14px",
        backgroundColor: "var(--toast-bg, #1e293b)",
        color: "var(--toast-fg, #f1f5f9)",
        borderRadius: 8,
        boxShadow: "0 10px 30px rgba(0,0,0,0.25)",
        fontSize: 13,
        lineHeight: 1.45,
      }}
    >
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start", gap: 12 }}>
        <div style={{ flex: 1 }}>
          <div style={{ fontWeight: 600, marginBottom: 2 }}>
            {summary.total} operation{summary.total === 1 ? "" : "s"} ran while you were away
          </div>
          {breakdown && (
            <div style={{ opacity: 0.85, fontSize: 12 }}>{breakdown}</div>
          )}
          {failed > 0 && (
            <div style={{ marginTop: 4, color: "var(--toast-warning, #fca5a5)", fontSize: 12 }}>
              {failed} failed — open the timeline for details
            </div>
          )}
        </div>
        <button
          onClick={() => {
            setVisible(false);
            onDismiss();
          }}
          aria-label="Dismiss"
          style={{
            background: "none",
            border: "none",
            color: "inherit",
            cursor: "pointer",
            fontSize: 16,
            lineHeight: 1,
            padding: 0,
            opacity: 0.7,
          }}
        >
          ×
        </button>
      </div>
    </div>
  );
}
