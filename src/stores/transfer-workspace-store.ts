import { create } from "zustand";
import { persist } from "zustand/middleware";
import { isBackendAvailable, tauriInvoke } from "@/hooks/use-tauri";
import { useFileManagerStore, paneLabel } from "@/stores/file-manager-store";
import { resolveItems, transferItems } from "@/lib/location-provider";
import { isRemotePath, parentLocation } from "@/lib/location";

export interface WorkspaceJob {
  id: string;
  source_path: string;
  dest_path: string;
  total_bytes: number;
  transferred_bytes: number;
  status: string;
  error_message: string | null;
  source_checksum?: string | null;
  dest_checksum?: string | null;
  speed_bps?: number;
  eta_seconds?: number | null;
  external?: boolean;
  status_available?: boolean;
}
export interface TransferSession {
  id: string;
  source: string;
  destination: string;
  sourceFolder: string;
  sources?: string[];
  destinationPaths?: string[];
  kept?: string[];
  mode: "copy" | "move";
  jobs: WorkspaceJob[];
  foldersCreated: number;
  movedCount: number;
  createdAt: string;
}
interface TransferWorkspaceState {
  sessions: TransferSession[];
  selectedId: string | null;
  select: (id: string | null) => void;
  add: (session: TransferSession) => void;
  updateJobs: (jobs: WorkspaceJob[]) => void;
}
export const useTransferWorkspace = create<TransferWorkspaceState>()(
  persist(
    (set) => ({
      sessions: [],
      selectedId: null,
      select: (selectedId) => set({ selectedId }),
      add: (session) =>
        set((s) => ({
          sessions: [session, ...s.sessions].slice(0, 50),
          selectedId: session.id,
        })),
      updateJobs: (jobs) =>
        set((s) => {
          const byId = new Map(jobs.map((job) => [job.id, job]));
          return {
            sessions: s.sessions.map((session) => ({
              ...session,
              jobs: session.jobs.map((job) => {
                const current = byId.get(job.id);
                if (current) return { ...current, status_available: true };
                return ["completed", "failed", "cancelled"].includes(job.status)
                  ? job
                  : { ...job, status_available: false };
              }),
            })),
          };
        }),
    }),
    { name: "ufop-transfer-workspace", version: 1 },
  ),
);

/** Accept a real queue identity, then hand off to the visible workspace.
 * Local jobs always run through the chunked, checksum-verified engine. */
export function queueWorkspaceTransfer(
  source: string,
  destination: string,
  mode: "copy" | "move",
) {
  return queueWorkspaceItems([source], destination, mode);
}

export async function queueWorkspaceItems(
  sources: string[],
  destination: string,
  mode: "copy" | "move",
): Promise<TransferSession> {
  if (!isBackendAvailable())
    throw new Error(
      "Open the desktop app to start a transfer. This browser view has no filesystem connection.",
    );
  if (!sources.length) throw new Error("Choose at least one source.");
  const source = sources[0];
  let jobs: WorkspaceJob[],
    foldersCreated: number,
    movedCount = 0;
  let sourceIsDir = false;
  let destinationPaths: string[] = [];
  let kept: string[] = [];
  if (sources.some(isRemotePath) || isRemotePath(destination)) {
    const items = await resolveItems(sources);
    sourceIsDir = items[0]?.is_dir ?? false;
    const out = await transferItems(items, destination, mode);
    jobs = out.jobs as WorkspaceJob[];
    foldersCreated = out.folders_created;
    movedCount = out.moved.length;
    kept = out.kept ?? [];
  } else {
    const meta = await tauriInvoke<{ is_dir: boolean }>("get_file_metadata", {
      path: source,
    });
    sourceIsDir = meta.is_dir;
    const out = await tauriInvoke<{
      jobs: WorkspaceJob[];
      folders_created: number;
      dest_paths: string[];
    }>("enqueue_local_transfer", {
      sourcePaths: sources,
      destDir: destination,
      mode,
    });
    if (
      !out ||
      !Array.isArray(out.jobs) ||
      !Array.isArray(out.dest_paths) ||
      out.dest_paths.length === 0
    )
      throw new Error(
        "The transfer was not accepted. Check both folders and try again.",
      );
    jobs = out.jobs;
    foldersCreated = out.folders_created;
    destinationPaths = out.dest_paths;
  }
  const sourceFolder =
    sources.length === 1 && sourceIsDir && mode === "copy"
      ? source
      : parentLocation(source);
  const session: TransferSession = {
    id: crypto.randomUUID(),
    source,
    sources,
    destination,
    sourceFolder,
    destinationPaths,
    kept,
    mode,
    jobs,
    foldersCreated,
    movedCount,
    createdAt: new Date().toISOString(),
  };
  useTransferWorkspace.getState().add(session);
  const files = useFileManagerStore.getState();
  if (files.singlePaneMode) files.toggleSinglePaneMode();
  for (const [index, path] of [
    [0, sourceFolder],
    [1, destination],
  ] as const) {
    const tab = files.getActiveTab(index);
    if (tab) files.navigateTab(index, tab.id, path, paneLabel(path));
  }
  window.dispatchEvent(new CustomEvent("ufop:show-workspace"));
  return session;
}
