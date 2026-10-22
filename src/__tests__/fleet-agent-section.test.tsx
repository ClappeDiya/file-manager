/**
 * "Managed by console": the store's IPC contract with the Rust
 * `fleet_agent_*` commands, the fallback outside Tauri, and the section's
 * register / claim / registered views.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { FleetAgentSection } from "@/components/fleet-agent-section";
import { UNREGISTERED, useFleetAgentStore, type AgentStatus } from "@/stores/fleet-agent-store";

const w = window as unknown as Record<string, unknown>;

const registered: AgentStatus = {
  ...UNREGISTERED,
  registered: true,
  enabled: true,
  console_url: "https://console.example.com",
  machine_id: "mch_1",
  organization: "Acme",
  machine_status: "active",
  heartbeat_seconds: 60,
  registered_via: "link",
  last_heartbeat_at: "2026-10-22T10:00:00Z",
  managed_policies: [{ policy_id: "fpl_1", job_id: "policy-fpl_1", version: 2, error: null }],
  pending_reports: 1,
};

describe("fleet agent", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    useFleetAgentStore.setState({ status: UNREGISTERED, busy: false, error: "" });
    w.__TAURI__ = {};
    w.__TAURI_INTERNALS__ = {};
  });

  afterEach(() => {
    delete w.__TAURI__;
    delete w.__TAURI_INTERNALS__;
  });

  it("falls back to an unregistered status outside Tauri", async () => {
    delete w.__TAURI__;
    delete w.__TAURI_INTERNALS__;
    await useFleetAgentStore.getState().refresh();
    expect(useFleetAgentStore.getState().status).toEqual(UNREGISTERED);
    expect(mockInvoke).not.toHaveBeenCalled();
  });

  it("registers with a link token", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => (cmd === "fleet_agent_register" ? registered : UNREGISTERED));
    render(<FleetAgentSection />);
    fireEvent.change(screen.getByLabelText("Console URL"), { target: { value: " https://console.example.com " } });
    fireEvent.change(screen.getByLabelText("Registration token"), { target: { value: "ufop_reg_abc" } });
    fireEvent.change(screen.getByLabelText("Tags"), { target: { value: "lab, linux" } });
    fireEvent.click(screen.getByRole("button", { name: "Register" }));
    await waitFor(() => expect(screen.getByTestId("fleet-agent-registered")).toBeTruthy());
    expect(mockInvoke).toHaveBeenCalledWith("fleet_agent_register", {
      consoleUrl: "https://console.example.com",
      registrationToken: "ufop_reg_abc",
      tags: "lab, linux",
    });
    expect(screen.getByText(/policy-fpl_1/)).toBeTruthy();
    expect(screen.getByText(/mch_1 · Acme/)).toBeTruthy();
  });

  it("shows the claim code and finishes when the poll registers", async () => {
    const claim = { console_url: "https://c.example", code: "ABCD-EFGH", expires_at: "2026-10-22T11:00:00Z" };
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "fleet_agent_start_claim") return claim;
      if (cmd === "fleet_agent_poll_claim") return { state: "registered", status: { ...registered, registered_via: "claim" } };
      return UNREGISTERED;
    });
    render(<FleetAgentSection />);
    fireEvent.click(screen.getByLabelText(/Claim code/));
    fireEvent.change(screen.getByLabelText("Console URL"), { target: { value: "https://c.example" } });
    fireEvent.click(screen.getByRole("button", { name: "Get claim code" }));
    await waitFor(() => expect(screen.getByTestId("claim-code").textContent).toBe("ABCD-EFGH"));
    expect(mockInvoke).toHaveBeenCalledWith("fleet_agent_start_claim", { consoleUrl: "https://c.example" });
    const st = await useFleetAgentStore.getState().pollClaim();
    expect(st).toBe("registered");
    await waitFor(() => expect(screen.getByTestId("fleet-agent-registered")).toBeTruthy());
  });

  it("surfaces AppError message and advice", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "fleet_agent_register") throw { error_type: "configuration", message: "The console rejected the request", advice: "Ask for a new link." };
      return UNREGISTERED;
    });
    const ok = await useFleetAgentStore.getState().register("https://c.example", "ufop_reg_x");
    expect(ok).toBe(false);
    expect(useFleetAgentStore.getState().error).toBe("The console rejected the request Ask for a new link.");
    expect(mockInvoke).toHaveBeenCalledWith("fleet_agent_register", { consoleUrl: "https://c.example", registrationToken: "ufop_reg_x", tags: null });
  });

  it("syncs, toggles and unregisters", async () => {
    useFleetAgentStore.setState({ status: registered });
    mockInvoke.mockImplementation(async (cmd: string, args?: { enabled?: boolean }) => {
      if (cmd === "fleet_agent_set_enabled") return { ...registered, enabled: !!args?.enabled };
      if (cmd === "fleet_agent_unregister") return UNREGISTERED;
      return registered;
    });
    vi.spyOn(window, "confirm").mockReturnValue(true);
    render(<FleetAgentSection />);
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("fleet_agent_status", undefined));
    fireEvent.click(screen.getByRole("button", { name: "Sync now" }));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("fleet_agent_sync_now", undefined));
    fireEvent.click(screen.getByRole("button", { name: "Turn off" }));
    await waitFor(() => expect(screen.getByText(/agent turned off/)).toBeTruthy());
    expect(mockInvoke).toHaveBeenCalledWith("fleet_agent_set_enabled", { enabled: false });
    fireEvent.click(screen.getByRole("button", { name: "Unregister" }));
    await waitFor(() => expect(screen.getByTestId("fleet-agent-unregistered")).toBeTruthy());
    expect(mockInvoke).toHaveBeenCalledWith("fleet_agent_unregister", { force: false });
  });
});
