import { describe, it, expect } from "vitest";
import {
  lightColors,
  darkColors,
  highContrastColors,
  spacing,
  semanticSpacing,
  fontSizes,
  fontFamilies,
  fontWeights,
  lineHeights,
  textStyles,
  shadows,
  darkShadows,
  radii,
  durations,
  easings,
  transitions,
} from "@ufop/design-tokens";

describe("@ufop/design-tokens", () => {
  describe("Color tokens", () => {
    it("should export light colors with all required keys", () => {
      expect(lightColors.primary).toBeDefined();
      expect(lightColors.bg).toBeDefined();
      expect(lightColors.text).toBeDefined();
      expect(lightColors.border).toBeDefined();
      expect(lightColors.success).toBeDefined();
      expect(lightColors.warning).toBeDefined();
      expect(lightColors.error).toBeDefined();
      expect(lightColors.info).toBeDefined();
      expect(lightColors.focusRing).toBeDefined();
    });

    it("should export dark colors with same shape as light", () => {
      const lightKeys = Object.keys(lightColors);
      const darkKeys = Object.keys(darkColors);
      expect(darkKeys).toEqual(lightKeys);
    });

    it("should export high contrast colors with same shape", () => {
      const lightKeys = Object.keys(lightColors);
      const hcKeys = Object.keys(highContrastColors);
      expect(hcKeys).toEqual(lightKeys);
    });

    it("light and dark colors should be different", () => {
      expect(lightColors.bg).not.toBe(darkColors.bg);
      expect(lightColors.text).not.toBe(darkColors.text);
    });

    it("high contrast text should be black for maximum contrast", () => {
      expect(highContrastColors.text).toBe("#000000");
    });

    it("all color values should be valid CSS color strings", () => {
      Object.values(lightColors).forEach((color) => {
        expect(typeof color).toBe("string");
        expect(
          color.startsWith("#") || color.startsWith("rgb"),
        ).toBe(true);
      });
    });
  });

  describe("Spacing tokens", () => {
    it("should export a complete spacing scale", () => {
      expect(spacing[0]).toBe("0px");
      expect(spacing[1]).toBe("4px");
      expect(spacing[2]).toBe("8px");
      expect(spacing[4]).toBe("16px");
      expect(spacing[8]).toBe("32px");
      expect(spacing[16]).toBe("64px");
    });

    it("should export semantic spacing values", () => {
      expect(semanticSpacing.inlineGap).toBeDefined();
      expect(semanticSpacing.componentPaddingX).toBeDefined();
      expect(semanticSpacing.sidebarWidth).toBeDefined();
      expect(semanticSpacing.toolbarHeight).toBeDefined();
      expect(semanticSpacing.fileRowHeight).toBeDefined();
    });

    it("spacing values should be valid CSS lengths", () => {
      Object.values(spacing).forEach((value) => {
        expect(value).toMatch(/^\d+px$/);
      });
    });
  });

  describe("Typography tokens", () => {
    it("should export font families", () => {
      expect(fontFamilies.sans).toContain("apple-system");
      expect(fontFamilies.mono).toContain("Mono");
    });

    it("should export font sizes from xs to 4xl", () => {
      expect(fontSizes.xs).toBe("0.75rem");
      expect(fontSizes.base).toBe("0.875rem");
      expect(fontSizes["4xl"]).toBe("1.875rem");
    });

    it("base font size should be 14px (0.875rem) for file manager", () => {
      expect(fontSizes.base).toBe("0.875rem");
    });

    it("should export font weights", () => {
      expect(fontWeights.normal).toBe("400");
      expect(fontWeights.medium).toBe("500");
      expect(fontWeights.semibold).toBe("600");
      expect(fontWeights.bold).toBe("700");
    });

    it("should export line heights", () => {
      expect(lineHeights.normal).toBe("1.5");
      expect(lineHeights.tight).toBe("1.25");
    });

    it("should export text styles with complete definitions", () => {
      expect(textStyles.heading1.fontSize).toBeDefined();
      expect(textStyles.heading1.fontWeight).toBeDefined();
      expect(textStyles.heading1.lineHeight).toBeDefined();
      expect(textStyles.body.fontSize).toBe(fontSizes.base);
      expect(textStyles.code.fontFamily).toBe("mono");
    });
  });

  describe("Shadow tokens", () => {
    it("should export shadows from none to xl plus inner", () => {
      expect(shadows.none).toBe("none");
      expect(shadows.xs).toBeDefined();
      expect(shadows.sm).toBeDefined();
      expect(shadows.md).toBeDefined();
      expect(shadows.lg).toBeDefined();
      expect(shadows.xl).toBeDefined();
      expect(shadows.inner).toContain("inset");
    });

    it("dark shadows should have higher opacity", () => {
      // Dark shadows use 0.2-0.3 opacity vs light 0.05-0.1
      expect(darkShadows.xs).toContain("0.2");
      expect(shadows.xs).toContain("0.05");
    });
  });

  describe("Border radius tokens", () => {
    it("should export radius scale", () => {
      expect(radii.none).toBe("0px");
      expect(radii.sm).toBe("4px");
      expect(radii.md).toBe("6px");
      expect(radii.lg).toBe("8px");
      expect(radii.full).toBe("9999px");
    });
  });

  describe("Animation tokens", () => {
    it("should export durations", () => {
      expect(durations.instant).toBe("0ms");
      expect(durations.fast).toBe("100ms");
      expect(durations.normal).toBe("200ms");
    });

    it("should export easings", () => {
      expect(easings.default).toContain("cubic-bezier");
      expect(easings.in).toContain("cubic-bezier");
      expect(easings.out).toContain("cubic-bezier");
    });

    it("should export predefined transitions", () => {
      expect(transitions.colors).toContain("color");
      expect(transitions.opacity).toContain("opacity");
      expect(transitions.transform).toContain("transform");
    });
  });
});
