import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { createElement } from "react";
import {
  ACCENT_STYLE_ID,
  accentCss,
  accentHex,
  accentPalette,
  applyAccentToDocument,
  hexToOklch,
  isAccent,
} from "@/lib/accent";
import { migrateUIState, useUIStore } from "@/stores/ui-store";
import { AppearanceSettings } from "@/components/appearance-settings";

const style = () => document.getElementById(ACCENT_STYLE_ID);

beforeEach(() => {
  applyAccentToDocument("blue");
  useUIStore.setState({ accentColor: "blue" });
});

describe("accent colour", () => {
  it("converts sRGB to OKLCH like Tailwind's palette", () => {
    // Tailwind 4 blue-600 is oklch(54.6% 0.245 262.881) = #155dfc; the
    // classic #2563eb is close to it.
    const { l, c, h } = hexToOklch("#155dfc");
    expect(l).toBeCloseTo(0.546, 2);
    expect(c).toBeCloseTo(0.245, 2);
    expect(h).toBeCloseTo(262.9, 0);
    expect(hexToOklch("#ffffff").l).toBeCloseTo(1, 3);
    expect(hexToOklch("#000000").l).toBeCloseTo(0, 3);
  });

  it("accepts presets and #rrggbb only", () => {
    expect(isAccent("teal")).toBe(true);
    expect(isAccent("#12ab9F")).toBe(true);
    expect(isAccent("#12ab9")).toBe(false);
    expect(isAccent("chartreuse")).toBe(false);
    expect(isAccent(3)).toBe(false);
    expect(accentHex("teal")).toBe("#0d9488");
    expect(accentHex("#ABCDEF")).toBe("#abcdef");
  });

  it("builds a ladder with the colour's hue and blue's lightness steps", () => {
    const p = accentPalette("teal");
    const tealHue = hexToOklch("#0d9488").h;
    expect(p[600]).toMatch(/^oklch\(54\.6% [0-9.]+ ([0-9.]+)\)$/);
    expect(Number(/ ([0-9.]+)\)$/.exec(p[600])![1])).toBeCloseTo(tealHue, 0);
    expect(p[50].startsWith("oklch(97.0%")).toBe(true);
    // A muted colour stays muted.
    const g = accentPalette("graphite");
    expect(Number(g[600].split(" ")[1])).toBeLessThan(0.08);
  });

  it("adds no stylesheet for the default blue and removes it when going back", () => {
    expect(accentCss("blue")).toBe("");
    expect(accentCss("#2563EB")).toBe("");
    applyAccentToDocument("rose");
    expect(style()?.textContent).toContain("--color-blue-600: oklch(");
    expect(style()?.textContent).toContain(':root[data-theme="dark"]');
    expect(style()?.textContent).toContain(':root:not([data-theme="high-contrast"])');
    applyAccentToDocument("violet");
    expect(document.querySelectorAll(`#${ACCENT_STYLE_ID}`)).toHaveLength(1);
    applyAccentToDocument("blue");
    expect(style()).toBeNull();
  });

  it("is a persisted UI setting; unknown values fall back to blue", () => {
    useUIStore.getState().setAccentColor("#ff8800");
    expect(useUIStore.getState().accentColor).toBe("#ff8800");
    expect(style()).not.toBeNull();
    useUIStore.getState().setAccentColor("not-a-colour");
    expect(useUIStore.getState().accentColor).toBe("blue");
    expect(style()).toBeNull();
    expect(migrateUIState({ accentColor: "nope", theme: "dark" }, 2)).toEqual({ theme: "dark" });
    expect(migrateUIState({ accentColor: "green" }, 2)).toEqual({ accentColor: "green" });
  });

  it("is chosen in Settings → Appearance", () => {
    render(createElement(AppearanceSettings));
    fireEvent.click(screen.getByTestId("accent-green"));
    expect(useUIStore.getState().accentColor).toBe("green");
    expect(screen.getByTestId("accent-green")).toHaveAttribute("aria-checked", "true");
    fireEvent.change(screen.getByTestId("accent-custom"), { target: { value: "#aa3366" } });
    expect(useUIStore.getState().accentColor).toBe("#aa3366");
  });
});
