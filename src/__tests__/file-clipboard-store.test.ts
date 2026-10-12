import { beforeEach, describe, expect, it } from "vitest";
import { useFileClipboardStore } from "@/stores/file-clipboard-store";

describe("file clipboard store", () => {
  beforeEach(() => useFileClipboardStore.getState().clear());
  it("records copy and cut selections", () => {
    useFileClipboardStore.getState().set(["/a", "/b"], "cut");
    expect(useFileClipboardStore.getState()).toMatchObject({ paths: ["/a", "/b"], mode: "cut" });
  });
  it("copies the array so later mutation cannot alter the clipboard", () => {
    const src = ["/a"];
    useFileClipboardStore.getState().set(src, "copy");
    src.push("/b");
    expect(useFileClipboardStore.getState().paths).toEqual(["/a"]);
  });
  it("clear empties it", () => {
    useFileClipboardStore.getState().set(["/a"], "copy");
    useFileClipboardStore.getState().clear();
    expect(useFileClipboardStore.getState().paths).toEqual([]);
  });
});
