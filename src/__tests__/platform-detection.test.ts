import { afterEach, expect, it, vi } from "vitest";
import { isWindowsPlatform } from "@/lib/copy-options";
afterEach(() => vi.unstubAllGlobals());
it.each([
  ["MacIntel", "Darwin", false],
  ["", "Mozilla/5.0 Darwin arm64", false],
  ["Win32", "", true],
  ["", "Mozilla/5.0 (Windows NT 10.0; Win64; x64)", true],
])("detects platform %s without treating Darwin as Windows", (platform, userAgent, expected) => {
  vi.stubGlobal("navigator", { platform, userAgent });
  expect(isWindowsPlatform()).toBe(expected);
});
