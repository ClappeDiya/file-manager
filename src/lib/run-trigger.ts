/**
 * What started a run, as the history views show it. Sync and backup runs
 * store a `RunTrigger` (Rust `ufop_core::run_trigger`); automation runs keep
 * the event that fired the rule (`trigger_event`: "manual", "schedule",
 * "file: <path>", "webhook"), which maps onto the same four words.
 */

export type RunTrigger = "manual" | "scheduled" | "watcher" | "api" | "automation";

const LABELS: Record<RunTrigger, string> = {
  manual: "Manual",
  scheduled: "Scheduled",
  watcher: "Watcher",
  api: "API",
  automation: "Automation",
};

/** A stored trigger for display; unknown values read as Manual (the column default). */
export function triggerLabel(trigger: string | null | undefined): string {
  return LABELS[(trigger ?? "manual") as RunTrigger] ?? LABELS.manual;
}

/** The trigger of an automation run, from its `trigger_event`. Events this
 *  does not know keep their own text, so nothing is mislabelled. */
export function automationTrigger(event: string | null | undefined): { label: string; detail: string | null } {
  const text = (event ?? "").trim();
  if (!text) return { label: "Manual", detail: null };
  const [kind, ...rest] = text.split(":");
  const detail = rest.join(":").trim() || null;
  switch (kind.trim()) {
    case "manual":
      return { label: "Manual", detail };
    case "schedule":
      return { label: "Scheduled", detail };
    case "file":
      return { label: "Watcher", detail };
    case "webhook":
      return { label: "API", detail };
    default:
      return { label: text, detail: null };
  }
}
