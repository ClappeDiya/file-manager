/**
 * EnginePulseIndicator — ambient operational heartbeat in the status bar.
 *
 * Polls `ledger_get_pulse` every 10 seconds to surface a compact summary
 * of recent cross-engine activity: total ops, active engines, error count,
 * and relative time since last activity.
 *
 * # Design principles
 *
 * - **Ambient, not intrusive.** A few tiny colored dots and terse text.
 *   Draws attention only when something interesting is happening (active
 *   ops) or something is wrong (errors). Otherwise shows "Idle".
 * - **Zero new infrastructure.** Composes the existing `since()` ledger
 *   query (one GROUP BY SQL scan per poll) via `getEnginePulse()`.
 * - **DRY visual grammar.** Uses the same CSS variables and icon scale
 *   as the surrounding status bar. No new design tokens.
 * - **Fail-safe.** Outside Tauri, the fallback is an empty summary and
 *   the indicator gracefully renders "Idle" with no error state.
 */
import { useEffect, useRef, useState } from "react";
import { Activity, AlertCircle } from "lucide-react";
import { getEnginePulse } from "@/lib/pulse";
import { formatBytesSuffix } from "@/lib/format-bytes";
import type { LedgerSinceSummary } from "@/components/since-last-seen-toast";
import { useActivityTimelineStore } from "@/stores/activity-timeline-store";

/** How often to poll (ms). 10 s keeps the status bar fresh without
 *  adding meaningful load — one lightweight SQL GROUP BY per tick. */
const POLL_INTERVAL_MS = 10_000;

/** Engine label map for human-readable tooltips. */
const ENGINE_LABELS: Record<string, string> = {
  fs: "Files",
  transfer: "Transfer",
  sync: "Sync",
  automation: "Automations",
  mount: "Mounts",
  spaces: "Spaces",
  ai: "AI",
  vault: "Vault",
  compat: "Compat",
  connector: "Connectors",
  system: "System",
};

/** Engine dot color map — matches existing palette conventions. */
const ENGINE_COLORS: Record<string, string> = {
  fs: "bg-blue-400",
  transfer: "bg-emerald-400",
  sync: "bg-violet-400",
  automation: "bg-amber-400",
  mount: "bg-cyan-400",
  spaces: "bg-pink-400",
  ai: "bg-indigo-400",
  vault: "bg-orange-400",
  compat: "bg-lime-400",
  connector: "bg-teal-400",
  system: "bg-gray-400",
};

export function EnginePulseIndicator() {
  const [pulse, setPulse] = useState<LedgerSinceSummary | null>(null);
  const mountedRef = useRef(true);
  // Drill-in: when the user clicks the error badge, open the Activity
  // Timeline pre-filtered to ALL failed events. Pulled via `.getState()`
  // inside the handler instead of via subscription because the
  // component does not need to re-render when the store mutates —
  // only the click consumer mutates it.
  //
  // The preset explicitly clears `engineFilter` (to `"all"`) and
  // `correlationFilter` (to `null`) so the user sees every failure,
  // not just failures inside whatever filter happened to be active
  // when they clicked. Omitting those fields would silently respect
  // the panel's prior state — surprising in the most common case of
  // "I just want to see what's broken".
  const openTimelineFailed = () => {
    useActivityTimelineStore.getState().openWithPreset({
      failedOnly: true,
      engineFilter: "all",
      correlationFilter: null,
    });
  };

  // Per-engine drill-in: clicking a colored dot opens the Activity
  // Timeline filtered to that engine. Symmetric with the error-badge
  // drill-in just above — same `openWithPreset` primitive, same
  // sibling-clearing intent (`failedOnly: false`, `correlationFilter:
  // null`) so a user clicking "Transfer" sees every transfer event,
  // not just transfer failures inside whatever filter happened to be
  // active. Together the badge + dots make every per-axis slice the
  // status bar already advertises one click away.
  const openTimelineForEngine = (engine: string) => {
    useActivityTimelineStore.getState().openWithPreset({
      engineFilter: engine,
      failedOnly: false,
      correlationFilter: null,
    });
  };

  useEffect(() => {
    mountedRef.current = true;
    const fetch = async () => {
      const result = await getEnginePulse();
      if (mountedRef.current) setPulse(result);
    };
    fetch();
    const id = setInterval(fetch, POLL_INTERVAL_MS);
    return () => {
      mountedRef.current = false;
      clearInterval(id);
    };
  }, []);

  if (!pulse) return null;

  const activeEngines = Object.keys(pulse.by_engine);
  const errorCount = pulse.by_status.failed ?? 0;
  const isActive = pulse.total > 0;

  // Per-engine tooltip text. Includes byte volume when the backend
  // computed it AND that engine actually moved data — silently omits
  // the `(0 B)` for engines whose events carry no bytes (renames,
  // folder creates) to keep the tooltip terse. `formatBytesSuffix`
  // centralises the suppression rule used here, in the engine-dot
  // button below, in the headline byte total, and in the
  // since-last-seen toast.
  const bytesByEngine = pulse.bytes_by_engine ?? null;
  const engineTooltip = activeEngines
    .map((e) => {
      const opCount = pulse.by_engine[e];
      const bytes = bytesByEngine ? bytesByEngine[e] : null;
      return `${ENGINE_LABELS[e] ?? e}: ${opCount}${formatBytesSuffix(bytes)}`;
    })
    .join(", ");

  // Global byte total — surfaced inline in the headline so the user
  // sees aggregate data volume alongside op count without scanning the
  // per-engine breakdown. Same suppression contract as above.
  const totalBytesLabel = formatBytesSuffix(pulse.bytes_total);

  const tooltip = isActive
    ? `${pulse.total} op${pulse.total !== 1 ? "s" : ""}${totalBytesLabel} in last 5 min — ${engineTooltip}`
    : "No recent activity";

  return (
    <div
      className="flex items-center gap-1.5"
      title={tooltip}
      role="status"
      aria-label={tooltip}
    >
      {/* Activity icon — pulses when active */}
      <Activity
        className={`h-3 w-3 ${
          isActive
            ? "text-emerald-500"
            : "text-[color:var(--color-text-tertiary)]"
        } ${isActive ? "animate-pulse" : ""}`}
        aria-hidden="true"
      />

      {isActive ? (
        <>
          {/* Compact op count */}
          <span className="text-[10px] tabular-nums text-[color:var(--color-text-secondary)]">
            {pulse.total}
          </span>

          {/* Engine dots — one per active engine. Each dot is a button
              whose VISUAL is still a tiny 6px circle (preserving the
              ambient aesthetic) but whose interactive area is padded to
              a generous click target via `p-1 -m-1`. Clicking opens
              the Activity Timeline filtered to that engine — same
              `openWithPreset` primitive the error badge uses. */}
          <div className="flex items-center gap-0.5">
            {activeEngines.map((engine) => {
              const label = ENGINE_LABELS[engine] ?? engine;
              const count = pulse.by_engine[engine];
              const engBytes = bytesByEngine ? bytesByEngine[engine] : null;
              const engBytesLabel = formatBytesSuffix(engBytes);
              return (
                <button
                  key={engine}
                  type="button"
                  onClick={() => openTimelineForEngine(engine)}
                  title={`${label}: ${count} op${count !== 1 ? "s" : ""}${engBytesLabel} — click to drill in`}
                  aria-label={`Open Activity Timeline filtered to ${label} (${count} op${count !== 1 ? "s" : ""}${engBytesLabel})`}
                  className="rounded-full p-1 -m-1 transition-colors hover:bg-[color:var(--color-hover-bg,rgba(255,255,255,0.08))] focus:outline-none focus:ring-1 focus:ring-sky-500/40"
                  data-testid={`engine-pulse-dot-${engine}`}
                >
                  <span
                    className={`block h-1.5 w-1.5 rounded-full ${ENGINE_COLORS[engine] ?? "bg-gray-400"}`}
                    aria-hidden="true"
                  />
                </button>
              );
            })}
          </div>

          {/* Error badge — only when errors exist. Clickable: opens
              the Activity Timeline pre-filtered to failed events so
              the user can drill in with one click instead of hunting
              for the failed-only chip inside the panel. The button
              shape and color are unchanged from the prior
              read-only badge so this is a purely accretive affordance. */}
          {errorCount > 0 && (
            <button
              type="button"
              onClick={openTimelineFailed}
              title={`${errorCount} failed op${errorCount !== 1 ? "s" : ""} in last 5 min — click to drill in`}
              aria-label={`Open Activity Timeline filtered to ${errorCount} failed event${errorCount !== 1 ? "s" : ""}`}
              className="flex items-center gap-0.5 rounded text-[10px] tabular-nums text-red-500 transition-colors hover:text-red-400 hover:bg-red-500/10 focus:outline-none focus:ring-1 focus:ring-red-500/40 px-1 -mx-1"
              data-testid="engine-pulse-error-badge"
            >
              <AlertCircle className="h-2.5 w-2.5" aria-hidden="true" />
              {errorCount}
            </button>
          )}
        </>
      ) : (
        <span className="text-[10px] text-[color:var(--color-text-tertiary)]">
          Idle
        </span>
      )}
    </div>
  );
}
