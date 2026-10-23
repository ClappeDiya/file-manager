/**
 * Settings → Logging: log retention and diagnostics bundles (save to a
 * folder; send to the console only when the machine is registered).
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { DiagnosticsSection } from "@/components/diagnostics-section";
import { DEFAULT_RETENTION } from "@/lib/diagnostics";
import { UNREGISTERED, useFleetAgentStore } from "@/stores/fleet-agent-store";

const w = window as unknown as Record<string, unknown>;

function backend(registered: boolean) {
  mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    switch (cmd) {
      case "get_log_retention":
        return { max_age_days: 7, max_total_mb: 50 };
      case "fleet_agent_status":
        return { ...UNREGISTERED, registered };
      case "set_log_retention":
        return 2;
      case "diagnostics_create":
        return { path: `${args?.outPath}/ufop-diagnostics-1.zip`, size: 4096, files: ["system.json", "logs/ufop.log"], truncated: false };
      case "diagnostics_send":
        return "dgn_1";
      default:
        throw new Error(`unexpected ${cmd}`);
    }
  });
}

describe("diagnostics section", () => {
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

  it("loads and saves the retention", async () => {
    backend(false);
    render(<DiagnosticsSection />);
    await waitFor(() => expect((screen.getByLabelText("Days to keep logs") as HTMLInputElement).value).toBe("7"));
    fireEvent.change(screen.getByLabelText("Days to keep logs"), { target: { value: "30" } });
    fireEvent.change(screen.getByLabelText("Maximum log size in MB"), { target: { value: "0" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(screen.getByText("Retention saved; removed 2 old log files")).toBeTruthy());
    expect(mockInvoke).toHaveBeenCalledWith("set_log_retention", { retention: { max_age_days: 30, max_total_mb: 1 } });
  });

  it("saves a bundle to a folder and hides sending when unregistered", async () => {
    backend(false);
    render(<DiagnosticsSection />);
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("fleet_agent_status", undefined));
    const save = screen.getByRole("button", { name: "Save diagnostics" }) as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    expect(screen.queryByRole("button", { name: "Send to console" })).toBeNull();
    fireEvent.change(screen.getByLabelText("Folder to save diagnostics"), { target: { value: " /tmp/out " } });
    fireEvent.click(save);
    await waitFor(() => expect(screen.getByText(/Saved \/tmp\/out\/ufop-diagnostics-1\.zip \(4 KB, 2 files\)/)).toBeTruthy());
    expect(mockInvoke).toHaveBeenCalledWith("diagnostics_create", { outPath: "/tmp/out" });
  });

  it("sends to the console when registered and shows errors", async () => {
    backend(true);
    render(<DiagnosticsSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Send to console" }));
    await waitFor(() => expect(screen.getByText("Diagnostics sent to the console (dgn_1)")).toBeTruthy());
    mockInvoke.mockRejectedValueOnce("The console is offline");
    fireEvent.click(screen.getByRole("button", { name: "Send to console" }));
    await waitFor(() => expect(screen.getByText("The console is offline")).toBeTruthy());
  });

  it("falls back to the default retention outside Tauri", async () => {
    delete w.__TAURI__;
    delete w.__TAURI_INTERNALS__;
    render(<DiagnosticsSection />);
    expect((screen.getByLabelText("Days to keep logs") as HTMLInputElement).value).toBe(String(DEFAULT_RETENTION.max_age_days));
    expect(mockInvoke).not.toHaveBeenCalled();
  });
});
