import { afterEach, describe, expect, it } from "vitest";
import { isTauriAvailable } from "@/hooks/use-tauri";

describe("isTauriAvailable", () => {
  afterEach(() => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    delete (window as unknown as Record<string, unknown>).__TAURI__;
  });

  it("is false in a plain browser", () => {
    expect(isTauriAvailable()).toBe(false);
  });

  it("is true when only __TAURI_INTERNALS__ is injected (withGlobalTauri off)", () => {
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    expect(isTauriAvailable()).toBe(true);
  });
});
