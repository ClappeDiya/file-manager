import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { resetInterfacePreferences, resetSettings, resetTips } from "@/lib/reset-settings";
import { ResetSettingsSection } from "@/components/reset-settings-section";
import { useUIStore } from "@/stores/ui-store";
import { useI18n } from "@/lib/i18n";
import { useFeaturePeekStore } from "@/stores/feature-peek-store";
import { useFileManagerStore } from "@/stores/file-manager-store";
import { ACCENT_STYLE_ID } from "@/lib/accent";

beforeEach(() => {
  mockInvoke.mockReset().mockResolvedValue({ removed: ["app.startup"] });
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  useUIStore.getState().setTheme("dark");
  useUIStore.getState().setAccentColor("rose");
  useUIStore.getState().setFontScale(1.25);
  useUIStore.setState({ toolbarItems: ["ai"], appMode: "advanced", onboardingComplete: true, confirmDragDrop: false, defaultDownloadDir: "/dl" });
  useI18n.setState({ preference: "de" });
  useFeaturePeekStore.setState({ dismissed: ["a", "b"] });
  useFileManagerStore.setState({ favorites: [{ path: "/home/me/Work", name: "Work" }] as never });
});

describe("Settings → Reset", () => {
  it("puts the interface back to a new install's defaults and keeps favorites and the tour state", () => {
    const d = useUIStore.getInitialState();
    resetInterfacePreferences();
    const s = useUIStore.getState();
    expect(s.theme).toBe(d.theme);
    expect(s.accentColor).toBe("blue");
    expect(document.getElementById(ACCENT_STYLE_ID)).toBeNull();
    expect(s.fontScale).toBe(1);
    expect(s.toolbarItems).toEqual(d.toolbarItems);
    expect(s.appMode).toBe(d.appMode);
    expect(s.confirmDragDrop).toBe(d.confirmDragDrop);
    expect(s.defaultDownloadDir).toBeNull();
    expect(useI18n.getState().preference).toBe("system");
    // Not part of this scope:
    expect(s.onboardingComplete).toBe(true);
    expect(useFileManagerStore.getState().favorites).toHaveLength(1);
  });

  it("shows the welcome tour and tips again", () => {
    resetTips();
    expect(useUIStore.getState().onboardingComplete).toBe(false);
    expect(useFeaturePeekStore.getState().dismissed).toEqual([]);
    expect(useUIStore.getState().theme).toBe("dark");
  });

  it("sends only the desktop scopes to the backend", async () => {
    await resetSettings(["interface", "transfers", "tips"]);
    expect(mockInvoke).toHaveBeenCalledWith("reset_settings", { scopes: ["transfers"] });
    mockInvoke.mockClear();
    expect(await resetSettings(["interface"])).toBe(0);
    expect(mockInvoke.mock.calls.filter((c) => c[0] === "reset_settings")).toEqual([]);
  });

  it("asks first; cancelling changes nothing", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(false);
    render(<ResetSettingsSection />);
    fireEvent.click(screen.getByTestId("reset-settings-button"));
    await waitFor(() => expect(confirm).toHaveBeenCalled());
    expect(confirm.mock.calls[0][0]).toMatch(/Kept: Connections, sync pairs/);
    expect(mockInvoke.mock.calls.filter((c) => c[0] === "reset_settings")).toEqual([]);
    expect(useUIStore.getState().accentColor).toBe("rose");
    confirm.mockRestore();
  });

  it("resets the chosen scopes after confirming", async () => {
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    const onReset = vi.fn();
    render(<ResetSettingsSection onReset={onReset} />);
    // Tips are not chosen by default; leave transfers out too.
    expect((screen.getByTestId("reset-scope-tips") as HTMLInputElement).checked).toBe(false);
    fireEvent.click(screen.getByTestId("reset-scope-transfers"));
    fireEvent.click(screen.getByTestId("reset-settings-button"));
    await waitFor(() => expect(onReset).toHaveBeenCalled());
    expect(mockInvoke).toHaveBeenCalledWith("reset_settings", { scopes: ["desktop"] });
    expect(useUIStore.getState().accentColor).toBe("blue");
    expect(screen.getByText("Settings reset to the defaults.")).toBeInTheDocument();
    confirm.mockRestore();
  });

  it("offers only the interface scopes outside the desktop app", () => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    render(<ResetSettingsSection />);
    expect(screen.getByTestId("reset-scope-interface")).toBeInTheDocument();
    expect(screen.queryByTestId("reset-scope-desktop")).toBeNull();
    expect(screen.queryByTestId("reset-scope-transfers")).toBeNull();
  });
});
