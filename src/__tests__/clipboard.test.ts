/**
 * clipboard — pure preview helper.
 *
 * Locks the truncation contract used by `copyToClipboardWithToast`
 * so the toast `why` line always renders to a predictable size,
 * regardless of how long the user's path / script / URL is.
 *
 * The side-effecting `copyToClipboardWithToast` itself is exercised
 * indirectly via the test-bench's jsdom Clipboard API in the
 * surrounding integration tests; this file owns the pure-helper
 * coverage.
 */
import { describe, it, expect } from "vitest";
import { previewClipboardValue } from "@/lib/clipboard";

describe("previewClipboardValue", () => {
  it("returns short strings verbatim", () => {
    expect(previewClipboardValue("hello")).toBe("hello");
  });

  it("collapses internal whitespace runs into single spaces", () => {
    expect(previewClipboardValue("a\n\nb\t  c")).toBe("a b c");
  });

  it("trims leading/trailing whitespace", () => {
    expect(previewClipboardValue("   /Users/me/file   ")).toBe("/Users/me/file");
  });

  it("middle-truncates long strings with an ellipsis", () => {
    const long = "a".repeat(50) + "/" + "b".repeat(50);
    const out = previewClipboardValue(long, 20);
    expect(out.length).toBe(20);
    expect(out).toContain("…");
  });

  it("preserves the start and end of long strings when middle-truncating", () => {
    const long =
      "/Users/me/Documents/very/deep/folder/structure/that/keeps/going/file.pdf";
    const out = previewClipboardValue(long, 30);
    expect(out.startsWith("/Users/me/")).toBe(true);
    // The end of the path (filename) is what the user usually
    // cares about — must survive middle truncation.
    expect(out.endsWith("file.pdf")).toBe(true);
    expect(out).toContain("…");
    expect(out.length).toBe(30);
  });

  it("returns an empty string when max is too small for an ellipsis", () => {
    expect(previewClipboardValue("anything", 0)).toBe("");
    expect(previewClipboardValue("anything", 3)).toBe("");
  });

  it("returns an empty string for whitespace-only input (after compaction)", () => {
    expect(previewClipboardValue("   \n\t  ")).toBe("");
  });

  it("returns empty string for empty input", () => {
    expect(previewClipboardValue("")).toBe("");
  });

  it("handles a value exactly at the max length without truncating", () => {
    const v = "x".repeat(60); // default max is 60
    expect(previewClipboardValue(v)).toBe(v);
    expect(previewClipboardValue(v)).toHaveLength(60);
  });

  it("collapses multi-line scripts into a single-line preview", () => {
    const script = ["cp -r a .", "cp -r b .", "cp -r c ."].join("\n");
    const out = previewClipboardValue(script);
    expect(out).not.toContain("\n");
    expect(out).toBe("cp -r a . cp -r b . cp -r c .");
  });
});
