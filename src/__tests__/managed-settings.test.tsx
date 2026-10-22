/**
 * Console-managed app settings: the blackout rule (same as the Rust one),
 * the updater honouring channel / auto-check / blackout, and the notice.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { describeManaged, inBlackout, type ManagedSettings } from "@/lib/managed-settings";
import { useUpdater } from "@/lib/updater";
import { ManagedSettingsNotice } from "@/components/managed-settings-notice";

const w = window as unknown as Record<string, unknown>;
// 2026-10-23 is a Friday.
const at = (day: number, hh: number, mm = 0) => new Date(2026, 9, 18 + day, hh, mm);

const managed: ManagedSettings = {
  version: 2,
  updates: { channel: "beta", autoCheck: false, blackout: { start: "08:00", end: "18:00", days: [1, 2, 3, 4, 5] } },
  proxy: { mode: "pac", url: "", pacUrl: "https://wpad/p.pac", noProxy: [] },
  ai: { enabled: null, allowCloud: false },
  transfers: { maxUploadKbps: 512, maxDownloadKbps: 0, maxConcurrentRuns: null },
};

describe("blackout window", () => {
  it("covers weekday office hours, end exclusive", () => {
    const b = { start: "08:00", end: "18:00", days: [1, 2, 3, 4, 5] };
    expect(inBlackout(b, at(1, 8))).toBe(true);
    expect(inBlackout(b, at(5, 17, 59))).toBe(true);
    expect(inBlackout(b, at(5, 18))).toBe(false);
    expect(inBlackout(b, at(0, 12))).toBe(false);
    expect(inBlackout(null, at(1, 9))).toBe(false);
  });

  it("assigns overnight windows to the day they start", () => {
    const b = { start: "22:00", end: "06:00", days: [5] };
    expect(inBlackout(b, at(5, 23))).toBe(true);
    expect(inBlackout(b, at(6, 5))).toBe(true);
    expect(inBlackout(b, at(6, 23))).toBe(false);
    expect(inBlackout(b, at(5, 5))).toBe(false);
  });

  it("describes what is managed", () => {
    expect(describeManaged(managed)).toEqual([
      "Update channel: beta",
      "Automatic update checks: off",
      "No updates 08:00–18:00 (Mon, Tue, Wed, Thu, Fri)",
      "Proxy: pac (https://wpad/p.pac)",
      "AI: local models only",
      "Total upload: 512 KB/s",
      "Total download: unlimited",
    ]);
    expect(describeManaged(null)).toEqual([]);
  });
});

describe("updater with managed settings", () => {
  beforeEach(() => {
    useUpdater.setState({ channel: "stable", autoCheck: true, managed: null, status: "idle", error: null, updateInfo: null });
  });

  it("forces the channel and auto-check and ignores user changes", () => {
    useUpdater.getState().applyManaged(managed.updates);
    expect(useUpdater.getState().channel).toBe("beta");
    expect(useUpdater.getState().autoCheck).toBe(false);
    useUpdater.getState().setChannel("stable");
    useUpdater.getState().setAutoCheck(true);
    expect(useUpdater.getState().channel).toBe("beta");
    expect(useUpdater.getState().autoCheck).toBe(false);
    useUpdater.getState().applyManaged(null);
    useUpdater.getState().setChannel("stable");
    expect(useUpdater.getState().channel).toBe("stable");
  });

  it("does not install inside the blackout window", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(at(2, 10));
    useUpdater.getState().applyManaged({ channel: null, autoCheck: null, blackout: { start: "08:00", end: "18:00", days: [] } });
    useUpdater.setState({ status: "available", updateInfo: { version: "9.9.9", date: "", body: "" } });
    await useUpdater.getState().downloadAndInstall();
    expect(useUpdater.getState().status).toBe("available");
    expect(useUpdater.getState().error).toBe("Updates are paused by your organization until 18:00");
    vi.useRealTimers();
  });
});

describe("managed settings notice", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
    w.__TAURI__ = {};
    w.__TAURI_INTERNALS__ = {};
  });
  afterEach(() => {
    delete w.__TAURI__;
    delete w.__TAURI_INTERNALS__;
  });

  it("lists managed settings and renders nothing when none", async () => {
    mockInvoke.mockResolvedValue(managed);
    const { unmount } = render(<ManagedSettingsNotice />);
    await waitFor(() => expect(screen.getByTestId("managed-settings-notice")).toBeTruthy());
    expect(mockInvoke).toHaveBeenCalledWith("managed_settings_get", undefined);
    expect(screen.getByText("AI: local models only")).toBeTruthy();
    unmount();
    mockInvoke.mockResolvedValue(null);
    render(<ManagedSettingsNotice />);
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledTimes(2));
    expect(screen.queryByTestId("managed-settings-notice")).toBeNull();
  });
});
