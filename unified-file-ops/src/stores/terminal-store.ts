/**
 * Terminal Store (T-046)
 *
 * Zustand store for built-in terminal state management.
 * Manages terminal sessions, split layouts, and persistence.
 */
import { create } from "zustand";
import { persist } from "zustand/middleware";
import { tauriInvoke } from "@/hooks/use-tauri";

// ── Types ──

export interface TerminalSession {
  id: string;
  title: string;
  session_type: "local" | "remote";
  shell: string;
  cwd: string;
  created_at: string;
  active: boolean;
  cols: number;
  rows: number;
}

export interface TerminalLayout {
  session_ids: string[];
  orientation: "horizontal" | "vertical";
  split_ratio: number;
}

// ── Store ──

interface TerminalState {
  // Sessions
  sessions: TerminalSession[];
  activeSessionId: string | null;

  // Layout
  layout: TerminalLayout | null;

  // UI state
  panelOpen: boolean;
  panelHeight: number;
  error: string | null;

  // Actions
  createLocalSession: (cwd?: string) => Promise<TerminalSession>;
  createRemoteSession: (params: {
    host: string;
    port: number;
    username: string;
    connectionId?: string;
  }) => Promise<TerminalSession>;
  closeSession: (sessionId: string) => Promise<void>;
  listSessions: () => Promise<void>;
  setActiveSession: (sessionId: string | null) => void;
  writeToSession: (sessionId: string, data: string) => Promise<void>;
  resizeSession: (sessionId: string, cols: number, rows: number) => Promise<void>;
  splitTerminal: (
    sessionIds: string[],
    orientation: "horizontal" | "vertical",
    ratio?: number
  ) => Promise<void>;
  getLayout: () => Promise<void>;
  escapePath: (path: string) => Promise<string>;
  getDefaultShell: () => Promise<string>;

  // UI actions
  setPanelOpen: (open: boolean) => void;
  togglePanel: () => void;
  setPanelHeight: (height: number) => void;
  clearError: () => void;
}

export const useTerminalStore = create<TerminalState>()(
  persist(
    (set) => ({
      // Initial state
      sessions: [],
      activeSessionId: null,
      layout: null,
      panelOpen: false,
      panelHeight: 300,
      error: null,

      // Actions
      createLocalSession: async (cwd?: string) => {
        try {
          const session = await tauriInvoke<TerminalSession>("terminal_create_local", {
            cwd: cwd || null,
            cols: 80,
            rows: 24,
          });
          set((state) => ({
            sessions: [...state.sessions, session],
            activeSessionId: session.id,
            panelOpen: true,
          }));
          return session;
        } catch (e: any) {
          set({ error: e?.message || "Failed to create terminal session" });
          throw e;
        }
      },

      createRemoteSession: async (params) => {
        try {
          const session = await tauriInvoke<TerminalSession>("terminal_create_remote", {
            host: params.host,
            port: params.port,
            username: params.username,
            connectionId: params.connectionId || null,
            cols: 80,
            rows: 24,
          });
          set((state) => ({
            sessions: [...state.sessions, session],
            activeSessionId: session.id,
            panelOpen: true,
          }));
          return session;
        } catch (e: any) {
          set({ error: e?.message || "Failed to create remote terminal session" });
          throw e;
        }
      },

      closeSession: async (sessionId: string) => {
        try {
          await tauriInvoke("terminal_close_session", { sessionId });
          set((state) => {
            const newSessions = state.sessions.filter((s) => s.id !== sessionId);
            const newActiveId =
              state.activeSessionId === sessionId
                ? newSessions[newSessions.length - 1]?.id ?? null
                : state.activeSessionId;
            return {
              sessions: newSessions,
              activeSessionId: newActiveId,
              panelOpen: newSessions.length > 0 ? state.panelOpen : false,
            };
          });
        } catch (e: any) {
          set({ error: e?.message || "Failed to close terminal session" });
        }
      },

      listSessions: async () => {
        try {
          const sessions = await tauriInvoke<TerminalSession[]>("terminal_list_sessions", undefined, []);
          set({ sessions });
        } catch (e: any) {
          set({ error: e?.message || "Failed to list sessions" });
        }
      },

      setActiveSession: (sessionId: string | null) => {
        set({ activeSessionId: sessionId });
      },

      writeToSession: async (sessionId: string, data: string) => {
        try {
          await tauriInvoke("terminal_write", { sessionId, data });
        } catch (e: any) {
          set({ error: e?.message || "Failed to write to terminal" });
        }
      },

      resizeSession: async (sessionId: string, cols: number, rows: number) => {
        try {
          await tauriInvoke("terminal_resize", { sessionId, cols, rows });
          set((state) => ({
            sessions: state.sessions.map((s) =>
              s.id === sessionId ? { ...s, cols, rows } : s
            ),
          }));
        } catch (e: any) {
          // Resize failures are non-critical
          console.warn("Terminal resize failed:", e);
        }
      },

      splitTerminal: async (
        sessionIds: string[],
        orientation: "horizontal" | "vertical",
        ratio = 0.5
      ) => {
        try {
          await tauriInvoke("terminal_set_layout", {
            sessionIds,
            orientation,
            splitRatio: ratio,
          });
          set({
            layout: {
              session_ids: sessionIds,
              orientation,
              split_ratio: ratio,
            },
          });
        } catch (e: any) {
          set({ error: e?.message || "Failed to set terminal layout" });
        }
      },

      getLayout: async () => {
        try {
          const layout = await tauriInvoke<TerminalLayout | null>("terminal_get_layout", undefined, null);
          set({ layout });
        } catch (e: any) {
          set({ error: e?.message || "Failed to get terminal layout" });
        }
      },

      escapePath: async (path: string) => {
        try {
          return await tauriInvoke<string>("terminal_escape_path", { path });
        } catch {
          // Fallback: basic escaping
          return path.replace(/ /g, "\\ ");
        }
      },

      getDefaultShell: async () => {
        try {
          return await tauriInvoke<string>("terminal_get_default_shell", undefined, "/bin/bash");
        } catch {
          return "/bin/bash";
        }
      },

      // UI actions
      setPanelOpen: (open: boolean) => set({ panelOpen: open }),
      togglePanel: () =>
        set((state) => ({ panelOpen: !state.panelOpen })),
      setPanelHeight: (height: number) =>
        set({ panelHeight: Math.max(150, Math.min(600, height)) }),
      clearError: () => set({ error: null }),
    }),
    {
      name: "ufop-terminal-state",
      partialize: (state) => ({
        panelOpen: state.panelOpen,
        panelHeight: state.panelHeight,
      }),
    },
  ),
);
