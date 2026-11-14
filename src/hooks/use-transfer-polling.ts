import { useEffect, useState } from "react";
import { isBackendAvailable, tauriInvoke } from "@/hooks/use-tauri";
import {
  useTransferWorkspace,
  type WorkspaceJob,
} from "@/stores/transfer-workspace-store";
import { dispatchRefresh } from "@/lib/refresh-affected";
import { errorText } from "@/lib/error-text";

/** One poller for the shell, including when the user navigates away. */
export function useTransferPolling() {
  const [jobs, setJobs] = useState<WorkspaceJob[]>([]);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!isBackendAvailable()) return;
    let stopped = false;
    let previous = new Map<string, string>();
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const current = await tauriInvoke<WorkspaceJob[]>("list_transfers");
        if (!stopped) {
          const changed = current.filter(
            (j) =>
              ["completed", "failed", "cancelled"].includes(j.status) &&
              previous.get(j.id) !== j.status,
          );
          if (changed.length)
            dispatchRefresh(
              changed.flatMap((j) => [j.source_path, j.dest_path]),
            );
          previous = new Map(current.map((j) => [j.id, j.status]));
          setJobs(current);
          useTransferWorkspace.getState().updateJobs(current);
          setError(null);
        }
      } catch (e) {
        if (!stopped)
          setError(
            `Transfer updates unavailable: ${errorText(e)}. Last known status is shown; reconnect to refresh.`,
          );
      }
      if (!stopped) timer = setTimeout(poll, 500);
    };
    void poll();
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, []);
  return { jobs, error };
}
