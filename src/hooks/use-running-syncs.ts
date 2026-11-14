import { useEffect, useState } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";
import type { OpItem, OpsSnapshot } from "@/stores/operations-store";

/** The syncs running now (from the operations snapshot), polled while mounted. */
export function useRunningSyncs(intervalMs: number): OpItem[] {
  const [syncs, setSyncs] = useState<OpItem[]>([]);
  useEffect(() => {
    let live = true;
    const tick = async () => {
      try {
        const snap = await tauriInvoke<OpsSnapshot>("operations_snapshot", { recentLimit: 0 }, { active: [], recent: [] });
        if (live) setSyncs((snap?.active ?? []).filter((i) => i.engine === "sync"));
      } catch {
        if (live) setSyncs([]);
      }
    };
    void tick();
    const t = setInterval(tick, intervalMs);
    return () => {
      live = false;
      clearInterval(t);
    };
  }, [intervalMs]);
  return syncs;
}
