import { describe, it, expect } from "vitest";
import {
  ENGINE_ICONS,
  ENGINE_LABELS,
  STATUS_META,
  ALL_ENGINE_KEYS,
} from "@/lib/engine-ui-constants";

describe("ENGINE_ICONS", () => {
  it("contains all 11 engine keys", () => {
    expect(Object.keys(ENGINE_ICONS)).toHaveLength(11);
  });

  it("every value is a lucide icon (forwardRef object)", () => {
    for (const Icon of Object.values(ENGINE_ICONS)) {
      expect(typeof Icon).toBe("object");
    }
  });
});

describe("ENGINE_LABELS", () => {
  it("has the same keys as ENGINE_ICONS", () => {
    expect(Object.keys(ENGINE_LABELS).sort()).toEqual(
      Object.keys(ENGINE_ICONS).sort(),
    );
  });

  it("every label is a non-empty string", () => {
    for (const label of Object.values(ENGINE_LABELS)) {
      expect(label.length).toBeGreaterThan(0);
    }
  });
});

describe("STATUS_META", () => {
  it("contains ok, failed, cancelled, skipped", () => {
    expect(Object.keys(STATUS_META).sort()).toEqual([
      "cancelled",
      "failed",
      "ok",
      "skipped",
    ]);
  });

  it("every entry has an Icon and a className", () => {
    for (const meta of Object.values(STATUS_META)) {
      expect(meta.Icon).toBeDefined();
      expect(typeof meta.className).toBe("string");
    }
  });
});

describe("ALL_ENGINE_KEYS", () => {
  it("matches the keys of ENGINE_ICONS", () => {
    expect(ALL_ENGINE_KEYS.sort()).toEqual(Object.keys(ENGINE_ICONS).sort());
  });
});
