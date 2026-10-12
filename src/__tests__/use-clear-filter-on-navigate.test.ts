import { beforeEach, describe, expect, it } from "vitest";
import { renderHook } from "@testing-library/react";
import { useClearFilterOnNavigate } from "@/hooks/use-clear-filter-on-navigate";
import { useFileManagerStore } from "@/stores/file-manager-store";

describe("useClearFilterOnNavigate", () => {
  beforeEach(() => useFileManagerStore.getState().setFilterText(0, ""));

  it("keeps the filter on first render", () => {
    useFileManagerStore.getState().setFilterText(0, "abc");
    renderHook(() => useClearFilterOnNavigate(0, "/a"));
    expect(useFileManagerStore.getState().panes[0].filterText).toBe("abc");
  });

  it("clears the filter when the path changes", () => {
    useFileManagerStore.getState().setFilterText(0, "abc");
    const { rerender } = renderHook(({ p }) => useClearFilterOnNavigate(0, p), { initialProps: { p: "/a" } });
    rerender({ p: "/b" });
    expect(useFileManagerStore.getState().panes[0].filterText).toBe("");
  });
});
