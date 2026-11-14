/**
 * K121: text size setting — store, root font scale, persistence, migration,
 * Ctrl/Cmd +/−/0 shortcuts, Settings → Appearance control and palette entries.
 */
import { beforeEach, describe, expect, it } from "vitest";
import { fireEvent, render, renderHook, screen } from "@testing-library/react";
import { FONT_SCALES, migrateUIState, nextFontScale, UI_STATE_VERSION, useUIStore } from "@/stores/ui-store";
import { textSizeStepForKey, useTextSizeShortcuts } from "@/hooks/use-text-size-shortcuts";
import { AppearanceSettings } from "@/components/appearance-settings";
import { getDefaultCommands } from "@/components/command-palette";

const scaleVar = () => document.documentElement.style.getPropertyValue("--ufop-font-scale");

beforeEach(() => {
  localStorage.clear();
  useUIStore.getState().setFontScale(1);
});

describe("font scale", () => {
  it("applies the scale to the root and clears it for the default", () => {
    useUIStore.getState().setFontScale(1.25);
    expect(useUIStore.getState().fontScale).toBe(1.25);
    expect(scaleVar()).toBe("1.25");
    useUIStore.getState().setFontScale(1);
    expect(scaleVar()).toBe("");
  });

  it("steps through the sizes and clamps at both ends", () => {
    expect(FONT_SCALES.map((f) => f.value)).toEqual([0.875, 1, 1.125, 1.25]);
    expect(nextFontScale(1, 1)).toBe(1.125);
    expect(nextFontScale(1.25, 1)).toBe(1.25);
    expect(nextFontScale(0.875, -1)).toBe(0.875);
    expect(nextFontScale(1.125, 0)).toBe(1);
    const s = useUIStore.getState();
    s.stepFontScale(1);
    s.stepFontScale(1);
    s.stepFontScale(1);
    expect(useUIStore.getState().fontScale).toBe(1.25);
    useUIStore.getState().stepFontScale(0);
    expect(useUIStore.getState().fontScale).toBe(1);
  });

  it("ignores values that are not a known size", () => {
    useUIStore.getState().setFontScale(3 as never);
    expect(useUIStore.getState().fontScale).toBe(1);
  });

  it("persists and is restored on rehydration", async () => {
    useUIStore.getState().setFontScale(1.125);
    const stored = JSON.parse(localStorage.getItem("ufop-ui-state") || "{}");
    expect(stored.state.fontScale).toBe(1.125);
    expect(stored.version).toBe(UI_STATE_VERSION);
    useUIStore.setState({ fontScale: 1 });
    localStorage.setItem("ufop-ui-state", JSON.stringify(stored));
    document.documentElement.style.removeProperty("--ufop-font-scale");
    await useUIStore.persist.rehydrate();
    expect(useUIStore.getState().fontScale).toBe(1.125);
    expect(scaleVar()).toBe("1.125");
  });

  it("migration drops an invalid stored size and keeps valid ones", () => {
    expect(migrateUIState({ fontScale: 7, backupAdvanced: null }, 1)).toEqual({ backupAdvanced: null });
    expect(migrateUIState({ fontScale: 1.25, backupAdvanced: null }, 1)).toEqual({ fontScale: 1.25, backupAdvanced: null });
  });
});

describe("shortcuts", () => {
  const k = (key: string, mods: Partial<KeyboardEvent> = { ctrlKey: true }) => ({ key, ctrlKey: false, metaKey: false, altKey: false, ...mods });

  it("maps Ctrl/Cmd + + − 0 and leaves Ctrl+= (compare) alone", () => {
    expect(textSizeStepForKey(k("+"))).toBe(1);
    expect(textSizeStepForKey(k("-", { metaKey: true }))).toBe(-1);
    expect(textSizeStepForKey(k("0"))).toBe(0);
    expect(textSizeStepForKey(k("="))).toBeNull();
    expect(textSizeStepForKey(k("+", {}))).toBeNull();
    expect(textSizeStepForKey(k("-", { ctrlKey: true, altKey: true }))).toBeNull();
  });

  it("the hook changes the text size on key presses", () => {
    renderHook(() => useTextSizeShortcuts());
    fireEvent.keyDown(window, { key: "+", ctrlKey: true });
    expect(useUIStore.getState().fontScale).toBe(1.125);
    fireEvent.keyDown(window, { key: "-", ctrlKey: true });
    fireEvent.keyDown(window, { key: "-", ctrlKey: true });
    expect(useUIStore.getState().fontScale).toBe(0.875);
    fireEvent.keyDown(window, { key: "0", metaKey: true });
    expect(useUIStore.getState().fontScale).toBe(1);
  });

  it("the palette (and so the cheat sheet) lists the shortcuts", () => {
    let got: number | null = null;
    const cmds = getDefaultCommands({ onTextSize: (s) => (got = s) });
    const larger = cmds.find((c) => c.id === "text-size-larger");
    expect(larger?.shortcut).toMatch(/\+$/);
    expect(cmds.find((c) => c.id === "text-size-default")?.shortcut).toMatch(/0$/);
    larger?.action();
    expect(got).toBe(1);
  });
});

describe("Settings → Appearance", () => {
  it("shows the sizes as a radio group and changes the setting", () => {
    render(<AppearanceSettings />);
    const group = screen.getByRole("radiogroup", { name: "Text size" });
    expect(group).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: /Default/ })).toHaveAttribute("aria-checked", "true");
    fireEvent.click(screen.getByRole("radio", { name: /Larger/ }));
    expect(useUIStore.getState().fontScale).toBe(1.25);
    expect(screen.getByRole("radio", { name: /Larger/ })).toHaveAttribute("aria-checked", "true");
  });
});
