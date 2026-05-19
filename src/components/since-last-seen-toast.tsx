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
 *
 * # Drill-in action
 *
 * Before this iteration the toast advertised "open the timeline for
 * details" but offered no action — the user had to remember the
 * keyboard shortcut. The body is now a button that opens the
 * Activity Timeline via the same `openWithPreset` primitive the
 * engine-pulse error badge uses, pre-filtered to failed events
 * whenever the summary contains any. The dismiss (×) button is
 * still a separate target so a user who wants the toast gone
 * without drilling in retains that option.
 */

import { useEffect, useState } from "react";
import { useActivityTimelineStore } from "@/stores/activity-timeline-store";
import { formatBytesSuffix } from "@/lib/format-bytes";

/** Backend payload — must match Rust `LedgerSinceSummary` exactly. */
export interface LedgerSinceSummary {
  since: string;
  total: number;
  by_engine: Record<string, number>;
  by_status: Record<string, number>;
  /** Total bytes moved across all events in the window, or `null` when
   *  the backend didn't compute it (first-run sentinel from
   *  `ledger_since_last_seen`). Surfaced by the Engine Pulse tooltip
   *  for at-a-glance data-volume context. Optional on the wire —
   *  older payloads omit it; renderers must tolerate undefined. */
  bytes_total?: number | null;
  /** Per-engine byte breakdown, mirroring `by_engine` shape. */
  bytes_by_engine?: Record<string, number> | null;
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
  // Total bytes moved while the user was away. Surfaced inline in the
  // headline ("12 ops · 1.2 GB ran while you were away") when present
  // and non-zero. Suppression rule lives in `formatBytesSuffix` so
  // first-run sentinel (null), older payloads (undefined), and
  // metadata-only windows (0) all collapse to the same empty string —
  // no noisy "0 B" label.
  const bytesSuffix = formatBytesSuffix(summary.bytes_total, " · ");

  // Drill-in: open the Activity Timeline pre-filtered to all failures
  // when the summary reports any, or just open the panel otherwise.
  // Explicitly clears any active engine/correlation narrowing so the
  // user sees the whole picture of "what happened while I was away"
  // — same intent contract as the engine-pulse error drill-in.
  const handleOpenTimeline = () => {
    useActivityTimelineStore.getState().openWithPreset({
      failedOnly: failed > 0,
      engineFilter: "all",
      correlationFilter: null,
    });
    setVisible(false);
    onDismiss();
  };

  // The drill-in CTA text mirrors the existing "open the timeline for
  // details" copy that already lived in the failed-line, so users who
  // read that hint find exactly the affordance it promised.
  const ctaText =
    failed > 0
      ? `Open Timeline — show ${failed} failed`
      : "Open Timeline";

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
        backgroundColor: "var(--toast-bg, #1e293b)",
        color: "var(--toast-fg, #f1f5f9)",
        borderRadius: 8,
        boxShadow: "0 10px 30px rgba(0,0,0,0.25)",
        fontSize: 13,
        lineHeight: 1.45,
      }}
    >
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start", gap: 12, padding: "12px 14px 0 14px" }}>
        <div style={{ flex: 1 }}>
          <div style={{ fontWeight: 600, marginBottom: 2 }}>
            {summary.total} operation{summary.total === 1 ? "" : "s"}
            {bytesSuffix} ran while you were away
          </div>
          {breakdown && (
            <div style={{ opacity: 0.85, fontSize: 12 }}>{breakdown}</div>
          )}
          {failed > 0 && (
            <div style={{ marginTop: 4, color: "var(--toast-warning, #fca5a5)", fontSize: 12 }}>
              {failed} failed
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
      <button
        onClick={handleOpenTimeline}
        aria-label={
          failed > 0
            ? `Open Activity Timeline filtered to ${failed} failed event${failed === 1 ? "" : "s"}`
            : "Open Activity Timeline"
        }
        data-testid="since-last-seen-open-timeline"
        style={{
          display: "block",
          marginTop: 8,
          width: "100%",
          padding: "8px 14px",
          background: "rgba(255,255,255,0.08)",
          border: "none",
          borderTop: "1px solid rgba(255,255,255,0.08)",
          borderBottomLeftRadius: 8,
          borderBottomRightRadius: 8,
          color: "inherit",
          cursor: "pointer",
          fontSize: 12,
          fontWeight: 500,
          textAlign: "left",
          fontFamily: "inherit",
        }}
      >
        {ctaText} →
      </button>
    </div>
  );
}
