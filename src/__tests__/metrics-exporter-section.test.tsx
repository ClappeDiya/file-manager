/** Desktop Prometheus exporter settings: off by default, save, token handling, errors, Advanced gate. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { MetricsExporterSection, portProblem, type MetricsStatus, type MetricsSettingsUpdate } from "@/components/metrics-exporter-section";
import { AdvancedOnly } from "@/components/backup/advanced-only";
import { useUIStore } from "@/stores/ui-store";

const off: MetricsStatus = { enabled: false, port: 9464, token_set: false, listening: false, url: null, error: null };
let current: MetricsStatus = off;

beforeEach(() => {
  current = { ...off };
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  mockInvoke.mockReset().mockImplementation(async (cmd: string, a: { settings: MetricsSettingsUpdate }) => {
    if (cmd === "desktop_metrics_status") return current;
    if (cmd === "desktop_metrics_configure") {
      const s = a.settings;
      if (s.port === 9999) throw { message: "Cannot listen for metrics on 127.0.0.1:9999: address in use", advice: "" };
      const token_set = s.token === null ? current.token_set : s.token !== "";
      current = { enabled: s.enabled, port: s.port ?? current.port, token_set, listening: s.enabled, url: s.enabled ? `http://127.0.0.1:${s.port}/metrics` : null, error: null };
      return current;
    }
    return null;
  });
});

describe("portProblem", () => {
  it("accepts 1024–65535 only", () => {
    expect(portProblem("9464")).toBeNull();
    expect(portProblem("80")).toMatch(/1024/);
    expect(portProblem("70000")).toMatch(/1024/);
    expect(portProblem("abc")).toMatch(/1024/);
  });
});

describe("MetricsExporterSection", () => {
  it("is off by default, then turns on with a port and a token", async () => {
    render(<MetricsExporterSection />);
    expect((await screen.findByTestId("metrics-state")).textContent).toBe("Off");
    fireEvent.click(screen.getByLabelText("Serve Prometheus metrics"));
    fireEvent.change(screen.getByLabelText("Metrics port"), { target: { value: "9500" } });
    fireEvent.change(screen.getByLabelText("Metrics token"), { target: { value: "scrape-token-1" } });
    fireEvent.click(screen.getByText("Save"));
    await waitFor(() => expect(screen.getByTestId("metrics-state").textContent).toBe("Serving http://127.0.0.1:9500/metrics (token required)"));
    expect(mockInvoke).toHaveBeenCalledWith("desktop_metrics_configure", { settings: { enabled: true, port: 9500, token: "scrape-token-1" } });
    // The saved token is kept unless removed explicitly.
    fireEvent.click(screen.getByText("Save"));
    await waitFor(() => expect(mockInvoke).toHaveBeenLastCalledWith("desktop_metrics_configure", { settings: { enabled: true, port: 9500, token: null } }));
    fireEvent.click(screen.getByLabelText("Remove the saved token"));
    fireEvent.click(screen.getByText("Save"));
    await waitFor(() => expect(mockInvoke).toHaveBeenLastCalledWith("desktop_metrics_configure", { settings: { enabled: true, port: 9500, token: "" } }));
    expect(screen.getByTestId("metrics-state").textContent).toBe("Serving http://127.0.0.1:9500/metrics");
  });

  it("refuses bad input before calling the backend and shows backend errors", async () => {
    render(<MetricsExporterSection />);
    await screen.findByTestId("metrics-state");
    fireEvent.change(screen.getByLabelText("Metrics port"), { target: { value: "80" } });
    fireEvent.click(screen.getByText("Save"));
    expect((await screen.findByRole("alert")).textContent).toMatch(/1024/);
    fireEvent.change(screen.getByLabelText("Metrics port"), { target: { value: "9464" } });
    fireEvent.change(screen.getByLabelText("Metrics token"), { target: { value: "short" } });
    fireEvent.click(screen.getByText("Save"));
    expect((await screen.findByRole("alert")).textContent).toMatch(/8 characters/);
    expect(mockInvoke.mock.calls.some(([c]) => c === "desktop_metrics_configure")).toBe(false);
    fireEvent.change(screen.getByLabelText("Metrics token"), { target: { value: "" } });
    fireEvent.click(screen.getByLabelText("Serve Prometheus metrics"));
    fireEvent.change(screen.getByLabelText("Metrics port"), { target: { value: "9999" } });
    fireEvent.click(screen.getByText("Save"));
    expect((await screen.findByRole("alert")).textContent).toMatch(/address in use/);
  });

  it("is behind the Advanced gate", () => {
    useUIStore.setState({ appMode: "simple", backupAdvanced: null });
    render(
      <AdvancedOnly hint>
        <MetricsExporterSection />
      </AdvancedOnly>,
    );
    expect(screen.queryByTestId("metrics-exporter")).toBeNull();
    fireEvent.click(screen.getByText("Switch to Advanced"));
    expect(screen.getByTestId("metrics-exporter")).toBeTruthy();
  });
});
