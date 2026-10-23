/**
 * K132: the Backup screens honour Simple/Advanced mode through the
 * `AdvancedOnly` gate, a per-area `backupAdvanced` override, and a persist
 * migration that keeps older installs on the full (advanced) backup UI.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { AdvancedOnly, useIsAdvanced } from "@/components/backup/advanced-only";
import { BackupSettingsView } from "@/components/backup/backup-settings-view";
import { migrateUIState, selectBackupAdvanced, UI_STATE_VERSION, useUIStore } from "@/stores/ui-store";

function Probe() {
  return <span data-testid="probe">{useIsAdvanced() ? "advanced" : "simple"}</span>;
}

beforeEach(() => {
  mockInvoke.mockReset();
  mockInvoke.mockResolvedValue([]);
  localStorage.clear();
  useUIStore.setState({ appMode: "simple", backupAdvanced: null });
});

describe("AdvancedOnly gate", () => {
  it("hides children in simple mode and shows them in advanced mode", () => {
    const { rerender } = render(<AdvancedOnly><p>expert</p></AdvancedOnly>);
    expect(screen.queryByText("expert")).toBeNull();
    expect(screen.queryByTestId("advanced-hint")).toBeNull();
    useUIStore.setState({ appMode: "advanced" });
    rerender(<AdvancedOnly><p>expert</p></AdvancedOnly>);
    expect(screen.getByText("expert")).toBeInTheDocument();
  });

  it("shows a hint in simple mode whose button switches to advanced", () => {
    useUIStore.setState({ backupAdvanced: false });
    render(<AdvancedOnly hint><p>expert</p></AdvancedOnly>);
    expect(screen.getByText(/More options are available in Advanced mode/)).toBeInTheDocument();
    fireEvent.click(screen.getByText("Switch to Advanced"));
    expect(useUIStore.getState().appMode).toBe("advanced");
    expect(useUIStore.getState().backupAdvanced).toBeNull();
    expect(screen.getByText("expert")).toBeInTheDocument();
    expect(screen.queryByTestId("advanced-hint")).toBeNull();
  });
});

describe("backupAdvanced override", () => {
  it("wins over appMode and null follows appMode", () => {
    expect(selectBackupAdvanced({ appMode: "simple", backupAdvanced: null })).toBe(false);
    expect(selectBackupAdvanced({ appMode: "advanced", backupAdvanced: null })).toBe(true);
    expect(selectBackupAdvanced({ appMode: "simple", backupAdvanced: true })).toBe(true);
    expect(selectBackupAdvanced({ appMode: "advanced", backupAdvanced: false })).toBe(false);
  });

  it("the hook reacts to the override", () => {
    render(<Probe />);
    expect(screen.getByTestId("probe")).toHaveTextContent("simple");
    act(() => useUIStore.getState().setBackupAdvanced(true));
    expect(screen.getByTestId("probe")).toHaveTextContent("advanced");
    act(() => {
      useUIStore.setState({ appMode: "advanced" });
      useUIStore.getState().setBackupAdvanced(false);
    });
    expect(screen.getByTestId("probe")).toHaveTextContent("simple");
  });

  it("settings toggle sets the override and clears it when it matches the app mode", () => {
    render(<BackupSettingsView />);
    const toggle = screen.getByTestId("backup-advanced-toggle") as HTMLInputElement;
    expect(toggle.checked).toBe(false);
    expect(screen.queryByText("Local databases")).toBeNull();
    fireEvent.click(toggle);
    expect(useUIStore.getState().backupAdvanced).toBe(true);
    expect(screen.getByText("Local databases")).toBeInTheDocument();
    fireEvent.click(toggle);
    // Unchecking matches the simple app mode, so the override goes back to null.
    expect(useUIStore.getState().backupAdvanced).toBeNull();
    expect(screen.queryByText("Local databases")).toBeNull();
  });

  it("persists the override", () => {
    useUIStore.getState().setBackupAdvanced(true);
    const stored = JSON.parse(localStorage.getItem("ufop-ui-state") || "{}");
    expect(stored.state.backupAdvanced).toBe(true);
    expect(stored.version).toBe(UI_STATE_VERSION);
  });
});

describe("persist migration", () => {
  it("migrateUIState turns on advanced backup UI for pre-v1 state only", () => {
    expect(migrateUIState({ appMode: "simple" }, 0)).toEqual({ appMode: "simple", backupAdvanced: true });
    expect(migrateUIState({ appMode: "simple", backupAdvanced: false }, 0)).toEqual({ appMode: "simple", backupAdvanced: false });
    expect(migrateUIState({ appMode: "simple", backupAdvanced: null }, 1)).toEqual({ appMode: "simple", backupAdvanced: null });
    expect(migrateUIState(undefined, 0)).toEqual({ backupAdvanced: true });
  });

  it("rehydrating an older install keeps every backup option visible", async () => {
    localStorage.setItem("ufop-ui-state", JSON.stringify({ state: { appMode: "simple", theme: "system" }, version: 0 }));
    await useUIStore.persist.rehydrate();
    expect(useUIStore.getState().backupAdvanced).toBe(true);
    expect(selectBackupAdvanced(useUIStore.getState())).toBe(true);
  });

  it("rehydrating current-version state leaves a null override alone", async () => {
    localStorage.setItem("ufop-ui-state", JSON.stringify({ state: { appMode: "simple", backupAdvanced: null }, version: UI_STATE_VERSION }));
    await useUIStore.persist.rehydrate();
    expect(useUIStore.getState().backupAdvanced).toBeNull();
  });

  it("a fresh install with nothing persisted follows the app mode", async () => {
    await useUIStore.persist.rehydrate();
    expect(useUIStore.getState().backupAdvanced).toBeNull();
    expect(selectBackupAdvanced(useUIStore.getState())).toBe(false);
  });
});
