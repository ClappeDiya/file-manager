/**
 * Engine Pulse — lightweight rolling-window query for the status bar.
 *
 * Calls `ledger_get_pulse` which runs one GROUP BY query over the last
 * N seconds of the operation ledger. Idempotent — safe to poll every
 * few seconds without losing event windows (unlike `ledger_since_last_seen`
 * which advances a stored timestamp).
 *
 * Reuses the existing {@link LedgerSinceSummary} type from the
 * since-last-seen toast so the wire format stays DRY.
 */
import { tauriInvokeSafe } from "@/hooks/use-tauri";
import type { LedgerSinceSummary } from "@/components/since-last-seen-toast";

/** Default rolling window: 5 minutes (300 seconds). */
const DEFAULT_WINDOW_SECS = 300;

/**
 * Fetch the engine pulse for the last `windowSecs` seconds.
 *
 * Fail-safe: returns an empty summary outside Tauri so the status bar
 * renders "Idle" gracefully in dev mode.
 */
export async function getEnginePulse(
  windowSecs: number = DEFAULT_WINDOW_SECS,
): Promise<LedgerSinceSummary> {
  const fallback: LedgerSinceSummary = {
    since: new Date().toISOString(),
    total: 0,
    by_engine: {},
    by_status: {},
  };
  return await tauriInvokeSafe<LedgerSinceSummary>(
    "ledger_get_pulse",
    { windowSecs },
    fallback,
  );
}
