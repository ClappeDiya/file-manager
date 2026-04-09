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
import type { LedgerSinceSummary } from "@/components/since-last-seen-toast";

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

  const engineTooltip = activeEngines
    .map((e) => `${ENGINE_LABELS[e] ?? e}: ${pulse.by_engine[e]}`)
    .join(", ");

  const tooltip = isActive
    ? `${pulse.total} op${pulse.total !== 1 ? "s" : ""} in last 5 min — ${engineTooltip}`
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

          {/* Engine dots — one per active engine */}
          <div className="flex items-center gap-0.5">
            {activeEngines.map((engine) => (
              <span
                key={engine}
                className={`inline-block h-1.5 w-1.5 rounded-full ${ENGINE_COLORS[engine] ?? "bg-gray-400"}`}
                title={`${ENGINE_LABELS[engine] ?? engine}: ${pulse.by_engine[engine]} ops`}
                aria-hidden="true"
              />
            ))}
          </div>

          {/* Error badge — only when errors exist */}
          {errorCount > 0 && (
            <span className="flex items-center gap-0.5 text-[10px] tabular-nums text-red-500">
              <AlertCircle className="h-2.5 w-2.5" aria-hidden="true" />
              {errorCount}
            </span>
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
