/**
 * Management console agent ("Managed by console"): registration with a
 * link token or a claim code, status, on/off and sync.
 *
 * Types mirror `ufop_core::agent::{AgentStatus, ClaimStatus,
 * ClaimPollResult}` (snake_case serde, `Option<T>` → `T | null`). Outside
 * Tauri every call resolves to an unregistered status.
 */
import { create } from "zustand";
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";

export interface ManagedPolicy {
  policy_id: string;
  job_id: string;
  version: number;
  error: string | null;
}

export interface ClaimStatus {
  console_url: string;
  code: string;
  expires_at: string;
}

export interface AgentStatus {
  registered: boolean;
  enabled: boolean;
  console_url: string;
  machine_id: string;
  organization: string;
  /** `active`, `suspended` or `revoked` (as last reported by the console). */
  machine_status: string;
  heartbeat_seconds: number;
  tags: string[];
  registered_at: string | null;
  /** `link` or `claim`. */
  registered_via: string;
  last_heartbeat_at: string | null;
  last_error: string | null;
  policy_set_version: string;
  managed_policies: ManagedPolicy[];
  pending_reports: number;
  dropped_reports: number;
  claim: ClaimStatus | null;
}

export interface ClaimPollResult {
  /** `pending`, `registered`, `expired` or `none`. */
  state: "pending" | "registered" | "expired" | "none";
  status: AgentStatus;
}

export const UNREGISTERED: AgentStatus = {
  registered: false,
  enabled: false,
  console_url: "",
  machine_id: "",
  organization: "",
  machine_status: "",
  heartbeat_seconds: 0,
  tags: [],
  registered_at: null,
  registered_via: "",
  last_heartbeat_at: null,
  last_error: null,
  policy_set_version: "",
  managed_policies: [],
  pending_reports: 0,
  dropped_reports: 0,
  claim: null,
};

/** AppError payloads are `{message, advice}`; anything else is stringified. */
export function describeAgentError(err: unknown): string {
  if (err && typeof err === "object" && "message" in err) {
    const { message, advice } = err as { message?: unknown; advice?: unknown };
    return [message, advice].filter((p) => typeof p === "string" && p).join(" ");
  }
  return String(err);
}

/** Mirrors `fleet_agent_commands::ConnectStart`. */
export interface ConnectStart {
  url: string;
  browser_opened: boolean;
}

interface FleetAgentState {
  status: AgentStatus;
  busy: boolean;
  error: string;
  refresh: () => Promise<void>;
  register: (consoleUrl: string, registrationToken: string, tags?: string) => Promise<boolean>;
  startClaim: (consoleUrl: string) => Promise<ClaimStatus | null>;
  pollClaim: () => Promise<ClaimPollResult["state"]>;
  cancelClaim: () => Promise<void>;
  /** Browser sign-in in progress (the console page to approve in). */
  connect: ConnectStart | null;
  /** Open the console in the browser and wait until the user approves. */
  signInWithBrowser: (consoleUrl: string, tags?: string) => Promise<boolean>;
  cancelSignIn: () => Promise<void>;
  unregister: (force?: boolean) => Promise<boolean>;
  setEnabled: (enabled: boolean) => Promise<void>;
  syncNow: () => Promise<void>;
}

export const useFleetAgentStore = create<FleetAgentState>((set, get) => {
  /** Run a call that returns a status; errors land in `error`. */
  const act = async (fn: () => Promise<AgentStatus>): Promise<boolean> => {
    set({ busy: true, error: "" });
    try {
      set({ status: await fn() });
      return true;
    } catch (err) {
      set({ error: describeAgentError(err) });
      return false;
    } finally {
      set({ busy: false });
    }
  };

  return {
    status: UNREGISTERED,
    busy: false,
    error: "",
    connect: null,

    refresh: async () => {
      set({ status: await tauriInvokeSafe<AgentStatus>("fleet_agent_status", undefined, UNREGISTERED) });
    },

    register: (consoleUrl, registrationToken, tags) =>
      act(() =>
        tauriInvoke<AgentStatus>("fleet_agent_register", {
          consoleUrl: consoleUrl.trim(),
          registrationToken: registrationToken.trim(),
          tags: tags?.trim() ? tags.trim() : null,
        }),
      ),

    startClaim: async (consoleUrl) => {
      set({ busy: true, error: "" });
      try {
        const claim = await tauriInvoke<ClaimStatus>("fleet_agent_start_claim", { consoleUrl: consoleUrl.trim() });
        set({ status: { ...get().status, claim } });
        return claim;
      } catch (err) {
        set({ error: describeAgentError(err) });
        return null;
      } finally {
        set({ busy: false });
      }
    },

    pollClaim: async () => {
      try {
        const r = await tauriInvoke<ClaimPollResult>("fleet_agent_poll_claim");
        set({ status: r.status, error: r.state === "expired" ? "The claim code expired; start again." : "" });
        return r.state;
      } catch (err) {
        set({ error: describeAgentError(err) });
        return "pending";
      }
    },

    cancelClaim: async () => {
      await act(() => tauriInvoke<AgentStatus>("fleet_agent_cancel_claim"));
    },

    signInWithBrowser: async (consoleUrl, tags) => {
      set({ error: "" });
      try {
        set({ connect: await tauriInvoke<ConnectStart>("fleet_agent_connect_start", { consoleUrl: consoleUrl.trim() }) });
      } catch (err) {
        set({ error: describeAgentError(err) });
        return false;
      }
      const ok = await act(() => tauriInvoke<AgentStatus>("fleet_agent_connect_finish", { tags: tags?.trim() ? tags.trim() : null }));
      set({ connect: null });
      return ok;
    },

    cancelSignIn: async () => {
      await tauriInvokeSafe("fleet_agent_connect_cancel", undefined, null);
      set({ connect: null });
    },

    unregister: (force = false) => act(() => tauriInvoke<AgentStatus>("fleet_agent_unregister", { force })),

    setEnabled: async (enabled) => {
      await act(() => tauriInvoke<AgentStatus>("fleet_agent_set_enabled", { enabled }));
    },

    syncNow: async () => {
      await act(() => tauriInvoke<AgentStatus>("fleet_agent_sync_now"));
    },
  };
});
