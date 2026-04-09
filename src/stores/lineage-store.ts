/**
 * Lineage Store
 *
 * Drives the "Show File History" right-side panel. Purely in-memory, not
 * persisted — a lineage request only makes sense in the moment the user
 * asks for it. Opening the panel replaces any previous request so the UI
 * never juggles stale history.
 *
 * Wire-format mirrors `FileLineage` in `src-tauri/src/lineage/mod.rs`.
 */
import { create } from "zustand";

/** Matches `LedgerEvent` in `src-tauri/src/ledger/mod.rs`. */
export interface LineageEvent {
  id: string;
  occurred_at: string;
  engine: string;
  kind: string;
  status: string;
  subject_path: string | null;
  target_path: string | null;
  bytes: number | null;
  correlation_id: string | null;
  summary: string;
  details_json: string;
  undo_token: string | null;
}

/** Matches `FileLineage` in `src-tauri/src/lineage/mod.rs`. */
export interface FileLineage {
  root_path: string;
  aliases: string[];
  events: LineageEvent[];
  correlation_ids: string[];
  truncated: boolean;
}

interface LineageState {
  /** Most recent request; `null` means the panel is closed. */
  request: FileLineage | null;
  /** True while a backend call is in flight for the currently requested path. */
  loading: boolean;
  /** Path currently being queried; persists across the loading window. */
  pendingPath: string | null;
  /** Last error string, if the backend rejected the request. */
  error: string | null;
  /** Open the panel for a specific path. Replaces any previous request. */
  beginRequest: (path: string) => void;
  /** Store a successful lineage payload from the backend. */
  setResult: (lineage: FileLineage) => void;
  /** Record a backend error. Clears loading. */
  setError: (message: string) => void;
  /** Close the panel and drop the current request. */
  close: () => void;
}

export const useLineageStore = create<LineageState>((set) => ({
  request: null,
  loading: false,
  pendingPath: null,
  error: null,
  beginRequest: (path) =>
    set({ request: null, loading: true, pendingPath: path, error: null }),
  setResult: (lineage) =>
    set({ request: lineage, loading: false, error: null }),
  setError: (message) =>
    set({ request: null, loading: false, error: message }),
  close: () =>
    set({ request: null, loading: false, pendingPath: null, error: null }),
}));
