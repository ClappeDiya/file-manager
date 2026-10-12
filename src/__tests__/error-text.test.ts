import { describe, expect, it } from "vitest";
import { errorText } from "@/lib/error-text";

describe("errorText", () => {
  it("reads Error instances and strings", () => {
    expect(errorText(new Error("boom"))).toBe("boom");
    expect(errorText("plain")).toBe("plain");
  });
  it("renders Tauri { message, advice } rejections instead of [object Object]", () => {
    expect(errorText({ message: "Not connected.", advice: "Connect first." })).toBe("Not connected. Connect first.");
    expect(errorText({ message: "Only message" })).toBe("Only message");
  });
  it("uses the fallback for unrecognised values", () => {
    expect(errorText({ code: 1 }, "Something failed")).toBe("Something failed");
  });
});
