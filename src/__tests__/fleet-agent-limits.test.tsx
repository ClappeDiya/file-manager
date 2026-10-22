/** Settings: the owner's limits on what the console may do on this machine. */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { FleetAgentLimits } from "@/components/fleet-agent-limits";

const w = window as unknown as Record<string, unknown>;

describe("fleet agent limits", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    w.__TAURI__ = {};
    w.__TAURI_INTERNALS__ = {};
  });
  afterEach(() => {
    delete w.__TAURI__;
    delete w.__TAURI_INTERNALS__;
  });

  it("loads, edits and saves the limits", async () => {
    mockInvoke.mockImplementation(async (cmd: string, args?: { limits: unknown }) =>
      cmd === "fleet_agent_set_limits" ? args!.limits : { roots: ["/data"], read_only: false, symlinks: "stay_inside", allow_scripts: true },
    );
    render(<FleetAgentLimits />);
    await waitFor(() => expect((screen.getByLabelText("Allowed folders") as HTMLTextAreaElement).value).toBe("/data"));
    fireEvent.change(screen.getByLabelText("Allowed folders"), { target: { value: " /data \n\n/srv/share " } });
    fireEvent.change(screen.getByLabelText("Symbolic links"), { target: { value: "follow" } });
    fireEvent.click(screen.getByLabelText(/Read-only/));
    fireEvent.click(screen.getByLabelText(/pre\/post scripts/));
    fireEvent.click(screen.getByRole("button", { name: "Save limits" }));
    await waitFor(() => expect(screen.getByText("Saved")).toBeTruthy());
    expect(mockInvoke).toHaveBeenCalledWith("fleet_agent_set_limits", {
      limits: { roots: ["/data", "/srv/share"], read_only: true, symlinks: "follow", allow_scripts: false },
    });
  });

  it("shows validation errors from the backend", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "fleet_agent_set_limits") throw { message: "'docs' is not an absolute folder", advice: "" };
      return { roots: [], read_only: false, symlinks: "stay_inside", allow_scripts: true };
    });
    render(<FleetAgentLimits />);
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("fleet_agent_get_limits", undefined));
    fireEvent.change(screen.getByLabelText("Allowed folders"), { target: { value: "docs" } });
    fireEvent.click(screen.getByRole("button", { name: "Save limits" }));
    await waitFor(() => expect(screen.getByText("'docs' is not an absolute folder")).toBeTruthy());
  });
});
