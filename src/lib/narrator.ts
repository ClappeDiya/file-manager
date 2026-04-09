/**
 * Operation Narrator — frontend wrapper around the Rust backend's
 * correlation-id narration command.
 *
 * Turns a raw list of ledger events sharing a correlation_id into a
 * plain-language story. The backend does all the work; this module
 * only provides a typed wrapper and a safe fallback so the browser
 * preview (outside Tauri) doesn't crash.
 */
import { tauriInvokeSafe } from "@/hooks/use-tauri";

/** Wire-format mirror of `narrator::NarrativeFact` in Rust. */
export interface NarrativeFact {
  tag: string;
  text: string;
}

/** Wire-format mirror of `narrator::OperationNarrative` in Rust. */
export interface OperationNarrative {
  correlation_id: string;
  event_count: number;
  engines_involved: string[];
  duration_ms: number;
  headline: string;
  story: string;
  facts: NarrativeFact[];
}

/**
 * Fetch the plain-language narrative for a correlation trace.
 *
 * Fail-safe: returns a minimal valid narrative (not null) if the backend
 * is unreachable — the UI never has to handle a null branch.
 */
export async function narrateOperation(
  correlationId: string,
): Promise<OperationNarrative> {
  const fallback: OperationNarrative = {
    correlation_id: correlationId,
    event_count: 0,
    engines_involved: [],
    duration_ms: 0,
    headline: "Narrator unavailable in preview mode.",
    story:
      "The operation narrator runs inside the Tauri shell. Launch the desktop app to see the cross-engine story for this correlation trace.",
    facts: [],
  };
  return await tauriInvokeSafe<OperationNarrative>(
    "narrator_narrate_operation",
    { correlationId },
    fallback,
  );
}
