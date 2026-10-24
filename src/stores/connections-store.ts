/**
 * Saved connections as the panes, sidebar and pickers see them: name,
 * protocol and the credentials-rejected flag, by id. Loaded on demand
 * (`ensureLoaded`) and reloaded when a connection is saved elsewhere
 * (`CONNECTIONS_CHANGED_EVENT`). Not persisted: the backend is the source
 * of truth. The Connections panel keeps its own full profiles.
 */
import { create } from "zustand";
import { tauriInvokeSafe } from "@/hooks/use-tauri";
import { CONNECTIONS_CHANGED_EVENT } from "@/lib/oauth";

/** The fields of `ConnectionProfile` (core/types.rs) the panes need. */
export interface PaneConnection {
  id: string;
  name: string;
  protocol: string;
  /** Where the connection opens (`remote_path`). */
  remote_path: string;
}

/** Mirrors `ConnectionStatus` (core/types.rs). */
export interface PaneConnectionStatus {
  id: string;
  starred: boolean;
  credentials_rejected_at: string | null;
  credentials_rejected_reason: string | null;
}

interface ConnectionsState {
  connections: PaneConnection[];
  statuses: Record<string, PaneConnectionStatus>;
  loaded: boolean;
  /** Reload the list and flags from the backend. */
  load: () => Promise<void>;
  /** Load once (first caller); later callers get the cached list. */
  ensureLoaded: () => Promise<void>;
  byId: (id: string) => PaneConnection | undefined;
  /** Display name of a connection id (undefined when unknown). */
  nameOf: (id: string) => string | undefined;
  /** Whether the service rejected this connection's credentials. */
  isRejected: (id: string) => boolean;
}

let inflight: Promise<void> | null = null;

export const useConnectionsStore = create<ConnectionsState>((set, get) => ({
  connections: [],
  statuses: {},
  loaded: false,
  load: async () => {
    const [conns, statuses] = await Promise.all([
      tauriInvokeSafe<PaneConnection[]>("list_connections", undefined, []),
      tauriInvokeSafe<PaneConnectionStatus[]>("list_connection_status", undefined, []),
    ]);
    set({
      connections: (conns ?? []).map((c) => ({ id: c.id, name: c.name, protocol: c.protocol, remote_path: c.remote_path || "/" })),
      statuses: Object.fromEntries((statuses ?? []).map((s) => [s.id, s])),
      loaded: true,
    });
  },
  ensureLoaded: () => {
    if (get().loaded) return Promise.resolve();
    inflight ??= get().load().finally(() => {
      inflight = null;
    });
    return inflight;
  },
  byId: (id) => get().connections.find((c) => c.id === id),
  nameOf: (id) => get().connections.find((c) => c.id === id)?.name,
  isRejected: (id) => !!get().statuses[id]?.credentials_rejected_at,
}));

if (typeof window !== "undefined") {
  window.addEventListener(CONNECTIONS_CHANGED_EVENT, () => {
    if (useConnectionsStore.getState().loaded) void useConnectionsStore.getState().load();
  });
}
