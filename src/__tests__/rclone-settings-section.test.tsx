import { describe, expect, it, vi, beforeEach, beforeAll } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { RcloneEngineInfo, RcloneSettingsSection } from "@/components/rclone/rclone-settings-section";
import { useUIStore } from "@/stores/ui-store";
import { rcloneStatus, type RcloneSettingsView, type RcloneStatus } from "@/lib/rclone";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

const STATUS: RcloneStatus = { path: "/usr/bin/rclone", source: "path", found: true, version: "v1.60.1", meets_minimum: true, min_version: "1.56.0", config_path: null, managed: false, error: null };

function view(patch: Partial<RcloneSettingsView> = {}, status: Partial<RcloneStatus> = {}): RcloneSettingsView {
  return {
    settings: { binary_path: null, config_path: null },
    status: { ...STATUS, ...status },
    default_config_path: "/home/u/.config/rclone/rclone.conf",
    managed_dir: "/data/ufop/rclone",
    pinned_version: "1.69.1",
    managed_version: null,
    managed_binary: null,
    update_available: false,
    platform_supported: true,
    ...patch,
  };
}

let current = view();
const calls = (cmd: string) => mockInvoke.mock.calls.filter((c) => c[0] === cmd).map((c) => c[1]);

beforeAll(() => {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

beforeEach(() => {
  current = view();
  mockInvoke.mockReset();
  mockInvoke.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
    switch (cmd) {
      case "rclone_settings_get":
        return Promise.resolve(current);
      case "rclone_settings_set": {
        const s = args?.settings as RcloneSettingsView["settings"];
        current = view({ settings: s }, { path: s.binary_path ?? STATUS.path, source: s.binary_path ? "settings" : "path", config_path: s.config_path });
        return Promise.resolve(current);
      }
      case "rclone_detect":
        return Promise.resolve([
          { path: "/usr/bin/rclone", origin: "path", version: "v1.60.1", meets_minimum: true, error: null },
          { path: "/opt/old/rclone", origin: "common", version: "v1.50.0", meets_minimum: false, error: null },
        ]);
      case "rclone_install_managed":
        return Promise.resolve({ result: { path: "/data/ufop/rclone/rclone", version: "v1.69.1", archive: "rclone-v1.69.1-linux-amd64.zip", sha256: "ab" }, view: view({ managed_version: "v1.69.1", managed_binary: "/data/ufop/rclone/rclone" }, { path: "/data/ufop/rclone/rclone", source: "settings", managed: true, version: "v1.69.1" }) });
      default:
        return Promise.resolve(undefined);
    }
  });
  useUIStore.setState({ appMode: "simple", backupAdvanced: null });
});

describe("Settings → rclone", () => {
  it("shows the binary in use, detects others and saves a choice", async () => {
    render(<RcloneSettingsSection />);
    expect(await screen.findByTestId("rclone-status")).toHaveTextContent("Using rclone v1.60.1 at /usr/bin/rclone (found on PATH)");
    expect(screen.getByTestId("rclone-status")).toHaveTextContent("/home/u/.config/rclone/rclone.conf");
    fireEvent.click(screen.getByText("Detect"));
    const list = await screen.findByTestId("rclone-candidates");
    expect(list).toHaveTextContent("too old");
    fireEvent.click(screen.getAllByText("Use")[1]);
    await waitFor(() => expect(calls("rclone_settings_set")).toEqual([{ settings: { binary_path: "/opt/old/rclone", config_path: null } }]));
    fireEvent.change(screen.getByLabelText(/rclone.conf/), { target: { value: "/etc/rclone.conf" } });
    fireEvent.click(screen.getByText("Save"));
    await waitFor(() => expect(calls("rclone_settings_set")[1]).toEqual({ settings: { binary_path: "/opt/old/rclone", config_path: "/etc/rclone.conf" } }));
  });

  it("shows a failed Detect as an error, not as 'No rclone found'", async () => {
    mockInvoke.mockImplementation((cmd: string) => (cmd === "rclone_detect" ? Promise.reject({ message: "Detect failed: permission denied", advice: "" }) : Promise.resolve(current)));
    render(<RcloneSettingsSection />);
    await screen.findByTestId("rclone-status");
    fireEvent.click(screen.getByText("Detect"));
    expect(await screen.findByText(/Detect failed: permission denied/)).toBeTruthy();
    expect(screen.queryByText("No rclone found on this computer.")).toBeNull();
  });

  it("warns about old and missing rclone", async () => {
    current = view({}, { version: "v1.50.0", meets_minimum: false });
    const { unmount } = render(<RcloneSettingsSection />);
    expect(await screen.findByTestId("rclone-min-warning")).toHaveTextContent("older than 1.56.0");
    unmount();
    current = view({}, { version: null, found: false, source: "none", path: "rclone", meets_minimum: null, error: "rclone is not installed" });
    render(<RcloneSettingsSection />);
    expect(await screen.findByText(/rclone is not installed/)).toBeTruthy();
  });

  it("offers the managed install only in Advanced mode", async () => {
    const { unmount } = render(<RcloneSettingsSection />);
    await screen.findByTestId("rclone-status");
    expect(screen.queryByTestId("rclone-managed")).toBeNull();
    unmount();
    useUIStore.setState({ appMode: "advanced" });
    render(<RcloneSettingsSection />);
    fireEvent.click(await screen.findByText("Install rclone 1.69.1"));
    expect(await screen.findByText(/Installed rclone v1.69.1 \(SHA-256 verified\)/)).toBeTruthy();
    expect(screen.getByTestId("rclone-status")).toHaveTextContent("managed copy");
    expect(screen.getByText("Up to date")).toBeDisabled();
  });

  // Regression: with a managed copy installed but another binary chosen,
  // the panel only showed a disabled "Up to date" and no way back to it.
  it("offers to switch back to an installed managed copy", async () => {
    useUIStore.setState({ appMode: "advanced" });
    current = view({ managed_version: "v1.69.1", managed_binary: "/data/ufop/rclone/rclone", settings: { binary_path: "/usr/bin/rclone", config_path: "/etc/rclone.conf" } }, { source: "settings", managed: false });
    render(<RcloneSettingsSection />);
    fireEvent.click(await screen.findByText("Use managed copy"));
    await waitFor(() => expect(calls("rclone_settings_set")).toEqual([{ settings: { binary_path: "/data/ufop/rclone/rclone", config_path: "/etc/rclone.conf" } }]));
  });

  it("does not offer the managed copy when it is already in use or missing", async () => {
    useUIStore.setState({ appMode: "advanced" });
    current = view({ managed_version: "v1.69.1", managed_binary: "/data/ufop/rclone/rclone" }, { path: "/data/ufop/rclone/rclone", source: "settings", managed: true, version: "v1.69.1" });
    const { unmount } = render(<RcloneSettingsSection />);
    await screen.findByTestId("rclone-managed");
    expect(screen.queryByText("Use managed copy")).toBeNull();
    unmount();
    current = view();
    render(<RcloneSettingsSection />);
    await screen.findByTestId("rclone-managed");
    expect(screen.queryByText("Use managed copy")).toBeNull();
  });

  // The read-only status call has a fallback (CLAUDE.md): a failing call
  // reads "status unavailable", never "not installed".
  it("About says the status is unavailable when it cannot be read", async () => {
    mockInvoke.mockImplementation(() => Promise.reject(new Error("ipc broke")));
    const s = await rcloneStatus();
    render(<RcloneEngineInfo status={s} />);
    expect(screen.getByTestId("about-rclone")).toHaveTextContent("rclone: status unavailable");
  });

  it("About shows the engine or 'not installed'", () => {
    const { rerender } = render(<RcloneEngineInfo status={STATUS} />);
    expect(screen.getByTestId("about-rclone")).toHaveTextContent("rclone: v1.60.1 (/usr/bin/rclone)");
    rerender(<RcloneEngineInfo status={{ ...STATUS, version: null, found: false, source: "none" }} />);
    expect(screen.getByTestId("about-rclone")).toHaveTextContent("rclone: not installed");
  });
});
