import { beforeEach, describe, expect, it } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { computeRangeSelection, useFileSelection } from "@/hooks/use-file-selection";
import { useFileManagerStore } from "@/stores/file-manager-store";
import type { FileEntryData } from "@/components/file-list";

const files = ["a", "b", "c", "d", "e"].map((n) => ({ path: `/x/${n}`, name: n })) as unknown as FileEntryData[];
const click = (o: Partial<React.MouseEvent> = {}) => ({ metaKey: false, ctrlKey: false, shiftKey: false, ...o }) as React.MouseEvent;

describe("computeRangeSelection", () => {
  const all = ["a", "b", "c", "d"];
  it("is inclusive and direction independent", () => {
    expect(computeRangeSelection(all, "b", "d")).toEqual(["b", "c", "d"]);
    expect(computeRangeSelection(all, "d", "b")).toEqual(["b", "c", "d"]);
  });
  it("falls back to the target when anchor is unknown", () => {
    expect(computeRangeSelection(all, null, "c")).toEqual(["c"]);
    expect(computeRangeSelection(all, "zz", "c")).toEqual(["c"]);
  });
});

describe("useFileSelection range behaviour", () => {
  beforeEach(() => useFileManagerStore.getState().setSelection("p", []));
  const sel = () => useFileManagerStore.getState().selectedPaths["p"];

  it("shift-click replaces the range from a fixed anchor", () => {
    const { result } = renderHook(() => useFileSelection("p", files));
    act(() => result.current.handleSelect("/x/b", click()));
    act(() => result.current.handleSelect("/x/e", click({ shiftKey: true })));
    expect(sel()).toEqual(["/x/b", "/x/c", "/x/d", "/x/e"]);
    act(() => result.current.handleSelect("/x/c", click({ shiftKey: true })));
    expect(sel()).toEqual(["/x/b", "/x/c"]);
  });

  it("shift+arrow extends from the anchor, plain arrow moves selection", () => {
    const { result } = renderHook(() => useFileSelection("p", files));
    act(() => result.current.handleKeyboardSelect("/x/a", false));
    act(() => result.current.handleKeyboardSelect("/x/b", true));
    act(() => result.current.handleKeyboardSelect("/x/c", true));
    expect(sel()).toEqual(["/x/a", "/x/b", "/x/c"]);
    act(() => result.current.handleKeyboardSelect("/x/d", false));
    expect(sel()).toEqual(["/x/d"]);
  });
});
