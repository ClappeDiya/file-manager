/**
 * K123: app-wide size units (binary / decimal) through formatBytes, and the
 * backup timestamp display (local / UTC) through formatDate.
 */
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { migrateUIState, useUIStore } from "@/stores/ui-store";
import { formatBytes, getDefaultByteUnits } from "@/lib/format-bytes";
import { formatFileSize } from "@/components/file-list";
import { formatFileSize as formatLocalized } from "@/lib/i18n";
import { formatBackupTime } from "@/lib/backup-time";
import { formatDate } from "@/lib/backup-helpers";
import { human } from "@/lib/mount-options";
import { AppearanceSettings } from "@/components/appearance-settings";

beforeEach(() => localStorage.clear());
afterEach(() => {
  useUIStore.getState().setByteUnits("decimal");
  useUIStore.getState().setTimeDisplay("local");
});

describe("size units", () => {
  it("defaults to decimal and switches every formatter to binary", () => {
    expect(useUIStore.getState().byteUnits).toBe("decimal");
    expect(formatBytes(1_500_000)).toBe("1.5 MB");
    useUIStore.getState().setByteUnits("binary");
    expect(getDefaultByteUnits()).toBe("binary");
    expect(formatBytes(1_572_864)).toBe("1.5 MiB");
    expect(formatFileSize(2048)).toBe("2.0 KiB");
    expect(human(2048)).toBe("2.0 KiB");
    expect(formatLocalized(2048, "en")).toBe("2.0 KiB");
    useUIStore.getState().setByteUnits("decimal");
    expect(formatFileSize(2000)).toBe("2.0 KB");
    expect(formatLocalized(2000, "en")).toBe("2.0 KB");
  });

  it("persists and applies the stored units on rehydration", async () => {
    useUIStore.getState().setByteUnits("binary");
    const stored = localStorage.getItem("ufop-ui-state")!;
    expect(JSON.parse(stored).state.byteUnits).toBe("binary");
    useUIStore.getState().setByteUnits("decimal");
    localStorage.setItem("ufop-ui-state", stored);
    await useUIStore.persist.rehydrate();
    expect(useUIStore.getState().byteUnits).toBe("binary");
    expect(formatBytes(1024)).toBe("1.0 KiB");
  });

  it("migration drops unknown units and time displays", () => {
    expect(migrateUIState({ byteUnits: "octal", timeDisplay: "mars", backupAdvanced: null }, 1)).toEqual({ backupAdvanced: null });
    expect(migrateUIState({ byteUnits: "binary", timeDisplay: "utc", backupAdvanced: null }, 1)).toEqual({ byteUnits: "binary", timeDisplay: "utc", backupAdvanced: null });
  });
});

describe("backup times", () => {
  it("shows local time by default and UTC when chosen", () => {
    const iso = "2026-03-16T00:30:00Z";
    expect(formatDate(iso)).toBe(new Date(iso).toLocaleString());
    useUIStore.getState().setTimeDisplay("utc");
    expect(formatDate(iso)).toBe(`${new Date(iso).toLocaleString(undefined, { timeZone: "UTC" })} UTC`);
    expect(formatBackupTime(null)).toBe("—");
    expect(formatBackupTime("not a time")).toBe("not a time");
    expect(formatBackupTime(iso, "local")).toBe(new Date(iso).toLocaleString());
  });
});

describe("Settings → Appearance", () => {
  it("offers size units and backup times as radio groups", () => {
    render(<AppearanceSettings />);
    const units = screen.getByRole("radiogroup", { name: "Size units" });
    expect(units).toBeInTheDocument();
    expect(screen.getByText(/1,500,000 bytes = 1\.5 MB/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("radio", { name: /Binary/ }));
    expect(useUIStore.getState().byteUnits).toBe("binary");
    expect(screen.getByText(/1,500,000 bytes = 1\.4 MiB/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("radio", { name: "UTC" }));
    expect(useUIStore.getState().timeDisplay).toBe("utc");
  });
});
