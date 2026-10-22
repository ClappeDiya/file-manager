/**
 * OS file drops: the pane under the (physical-pixel) cursor is highlighted
 * and receives a copy; drops elsewhere or of the pane's own files do nothing.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { OS_DROP_ATTR, OS_DROP_HOVER_ATTR, dropTargetAt, handleOsDrop } from "@/hooks/use-os-file-drop";

let pane: HTMLElement;
let inner: HTMLElement;
const at = vi.fn<(x: number, y: number) => Element | null>();

beforeEach(() => {
  mockInvoke.mockReset().mockResolvedValue(undefined);
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  document.body.innerHTML = `<div id="pane" ${OS_DROP_ATTR}="/home/me/Docs"><span id="row">x</span></div><div id="other"></div>`;
  pane = document.getElementById("pane")!;
  inner = document.getElementById("row")!;
  at.mockReset().mockImplementation((x) => (x < 100 ? inner : document.getElementById("other")));
  document.elementFromPoint = at as unknown as typeof document.elementFromPoint;
});

describe("OS file drop", () => {
  it("maps physical pixels to CSS pixels to find the pane", () => {
    expect(dropTargetAt({ x: 150, y: 10 }, 2)).toBe(pane);
    expect(at).toHaveBeenCalledWith(75, 5);
    expect(dropTargetAt({ x: 150, y: 10 }, 1)).toBeNull();
  });

  it("highlights the hovered pane and clears it on leave", async () => {
    await handleOsDrop({ type: "over", position: { x: 10, y: 10 } }, 1);
    expect(pane.hasAttribute(OS_DROP_HOVER_ATTR)).toBe(true);
    await handleOsDrop({ type: "over", position: { x: 500, y: 10 } }, 1);
    expect(pane.hasAttribute(OS_DROP_HOVER_ATTR)).toBe(false);
    await handleOsDrop({ type: "over", position: { x: 10, y: 10 } }, 1);
    await handleOsDrop({ type: "leave" }, 1);
    expect(pane.hasAttribute(OS_DROP_HOVER_ATTR)).toBe(false);
  });

  it("copies dropped files into the pane's folder", async () => {
    const r = await handleOsDrop({ type: "drop", paths: ["/tmp/a.txt", "/home/me/Docs/self.txt"], position: { x: 10, y: 10 } }, 1);
    expect(r).toEqual({ dest: "/home/me/Docs", paths: ["/tmp/a.txt"] });
    expect(mockInvoke).toHaveBeenCalledWith("copy_files", { sourcePaths: ["/tmp/a.txt"], destDir: "/home/me/Docs" });
    expect(pane.hasAttribute(OS_DROP_HOVER_ATTR)).toBe(false);
  });

  it("ignores drops outside panes", async () => {
    expect(await handleOsDrop({ type: "drop", paths: ["/tmp/a"], position: { x: 500, y: 0 } }, 1)).toBeNull();
    expect(mockInvoke).not.toHaveBeenCalled();
  });
});
