/** Operations drawer: what's running across engines (mirrors operations_commands.rs). */
import { create } from "zustand";
import { tauriInvoke } from "@/hooks/use-tauri";

export interface OpItem {
  engine: "transfer" | "sync" | "backup" | "index" | string;
  id: string;
  title: string;
  detail: string | null;
  state: "running" | "queued" | "paused" | string;
  fraction: number | null;
  bytes_done: number;
  bytes_total: number;
  items_done: number;
  items_total: number;
  started_at: string | null;
  bytes_per_sec: number;
  eta_secs: number | null;
  cancellable: boolean;
}

export interface LedgerEventLite {
  id: string;
  occurred_at: string;
  engine: string;
  kind: string;
  status: string;
  summary: string;
  subject_path: string | null;
}

export interface OpsSnapshot {
  active: OpItem[];
  recent: LedgerEventLite[];
}

export const operations = {
  snapshot: (recentLimit: number) => tauriInvoke<OpsSnapshot>("operations_snapshot", { recentLimit }),
  cancel: (engine: string, id: string) => tauriInvoke<boolean>("operations_cancel", { engine, id }),
};

interface OperationsState {
  open: boolean;
  toggle: () => void;
  setOpen: (open: boolean) => void;
}

export const useOperationsStore = create<OperationsState>((set) => ({
  open: false,
  toggle: () => set((s) => ({ open: !s.open })),
  setOpen: (open) => set({ open }),
}));

/** "1m 05s" style ETA. */
export function formatEta(secs: number | null): string | null {
  if (secs === null || secs < 0) return null;
  if (secs < 60) return `${secs}s`;
  const m = Math.floor(secs / 60);
  if (m < 60) return `${m}m ${String(secs % 60).padStart(2, "0")}s`;
  return `${Math.floor(m / 60)}h ${String(m % 60).padStart(2, "0")}m`;
}
