/**
 * Smart Spaces Store
 *
 * Zustand store for Smart Spaces — named workspaces bundling
 * local folder + remote connection + sync pair + automations.
 */
import { create } from "zustand";
import { persist } from "zustand/middleware";
import { tauriInvoke } from "@/hooks/use-tauri";

// ── Types (mirrors Rust serde output) ──

export interface SmartSpace {
  id: string;
  name: string;
  icon: string;
  color: string;
  local_path: string;
  connection_id: string | null;
  remote_path: string | null;
  sync_pair_id: string | null;
  enabled: boolean;
  created_at: string;
  updated_at: string;
}

export type SpaceStatus =
  | { type: "local_only" }
  | { type: "offline" }
  | { type: "connected" }
  | { type: "syncing" }
  | { type: "synced"; last_sync_at: string }
  | { type: "conflicts"; count: number }
  | { type: "error"; message: string };

export interface SpaceStatusResult {
  space_id: string;
  status: SpaceStatus;
  connection_alive: boolean;
  last_sync_at: string | null;
  conflict_count: number;
}

// ── Store ──

interface SpacesState {
  // Data (loaded from backend, not persisted in localStorage)
  spaces: SmartSpace[];
  statusMap: Record<string, SpaceStatusResult>;
  loading: boolean;

  // UI state (persisted)
  wizardOpen: boolean;
  prefillLocalPath: string | null;
  selectedSpaceId: string | null;

  // UI actions
  openWizard: (prefillPath?: string) => void;
  closeWizard: () => void;
  selectSpace: (id: string | null) => void;

  // Data actions
  loadSpaces: () => Promise<void>;
  createSpace: (space: SmartSpace) => Promise<SmartSpace | null>;
  updateSpace: (space: SmartSpace) => Promise<void>;
  deleteSpace: (id: string) => Promise<void>;
  activateSpace: (id: string) => Promise<SmartSpace | null>;
  refreshStatus: (id: string) => Promise<void>;
  refreshAllStatuses: () => Promise<void>;
}

export const useSpacesStore = create<SpacesState>()(
  persist(
    (set, get) => ({
      spaces: [],
      statusMap: {},
      loading: false,
      wizardOpen: false,
      prefillLocalPath: null,
      selectedSpaceId: null,

      openWizard: (prefillPath) =>
        set({ wizardOpen: true, prefillLocalPath: prefillPath ?? null }),
      closeWizard: () => set({ wizardOpen: false, prefillLocalPath: null }),
      selectSpace: (id) => set({ selectedSpaceId: id }),

      loadSpaces: async () => {
        set({ loading: true });
        try {
          const spaces = await tauriInvoke<SmartSpace[]>(
            "list_spaces",
            {},
            [],
          );
          set({ spaces, loading: false });
        } catch {
          set({ loading: false });
        }
      },

      createSpace: async (space) => {
        try {
          const created = await tauriInvoke<SmartSpace>("create_space", {
            space,
          });
          await get().loadSpaces();
          return created;
        } catch {
          return null;
        }
      },

      updateSpace: async (space) => {
        try {
          await tauriInvoke("update_space", { space });
          await get().loadSpaces();
        } catch {
          // silently fail — frontend cache stays as-is
        }
      },

      deleteSpace: async (id) => {
        try {
          await tauriInvoke("delete_space", { spaceId: id });
          set((s) => ({
            spaces: s.spaces.filter((sp) => sp.id !== id),
            selectedSpaceId: s.selectedSpaceId === id ? null : s.selectedSpaceId,
          }));
        } catch {
          // silently fail
        }
      },

      activateSpace: async (id) => {
        try {
          const space = await tauriInvoke<SmartSpace>("activate_space", {
            spaceId: id,
          });
          set({ selectedSpaceId: id });
          return space;
        } catch {
          return null;
        }
      },

      refreshStatus: async (id) => {
        try {
          const result = await tauriInvoke<SpaceStatusResult>(
            "get_space_status",
            { spaceId: id },
          );
          set((s) => ({
            statusMap: { ...s.statusMap, [id]: result },
          }));
        } catch {
          // best-effort
        }
      },

      refreshAllStatuses: async () => {
        const { spaces, refreshStatus } = get();
        await Promise.allSettled(spaces.map((s) => refreshStatus(s.id)));
      },
    }),
    {
      name: "ufop-spaces",
      partialize: (state) => ({
        wizardOpen: state.wizardOpen,
        selectedSpaceId: state.selectedSpaceId,
      }),
    },
  ),
);
