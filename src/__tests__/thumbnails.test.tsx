/** Grid thumbnails: LRU, request sharing, cell rendering, fallbacks. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...a: unknown[]) => mockInvoke(...a),
  convertFileSrc: (p: string) => `asset://localhost${p}`,
}));

import { GridIcon, type FileEntryData } from "@/components/file-list";
import { LruMap, clearThumbnailMemory, hasThumbnail, loadThumbnail } from "@/lib/thumbnails";

const entry = (name: string, is_dir = false): FileEntryData => ({
  name, path: `/pics/${name}`, is_dir, is_symlink: false, size: 10, modified: "2026-01-01T00:00:00Z", created: null, is_hidden: false, extension: null, permissions: null,
});

beforeEach(() => {
  clearThumbnailMemory();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  mockInvoke.mockReset().mockImplementation(async (_c: string, a: { path: string }) => (a.path.endsWith("broken.jpg") ? null : `/cache/${a.path.split("/").pop()}.png`));
});

describe("LruMap", () => {
  it("evicts the least recently used", () => {
    const m = new LruMap<string, number>(2);
    m.set("a", 1);
    m.set("b", 2);
    m.get("a");
    m.set("c", 3);
    expect([m.has("a"), m.has("b"), m.has("c"), m.size]).toEqual([true, false, true, 2]);
  });
});

describe("thumbnails", () => {
  it("knows which files have one", () => {
    expect([hasThumbnail("a.JPG"), hasThumbnail("b.webp"), hasThumbnail("c.pdf"), hasThumbnail("noext")]).toEqual([true, true, false, false]);
  });

  it("shares in-flight requests and caches results", async () => {
    const [a, b] = await Promise.all([loadThumbnail("/pics/x.png", 96), loadThumbnail("/pics/x.png", 96)]);
    expect(a).toBe("asset://localhost/cache/x.png.png");
    expect(b).toBe(a);
    await loadThumbnail("/pics/x.png", 96);
    expect(mockInvoke).toHaveBeenCalledTimes(1);
    await loadThumbnail("/pics/x.png", 96, "v2");
    expect(mockInvoke).toHaveBeenCalledTimes(2);
  });

  it("renders a thumbnail for images and the glyph otherwise", async () => {
    const { container } = render(
      <div>
        <GridIcon entry={entry("cat.png")} />
        <GridIcon entry={entry("broken.jpg")} />
        <GridIcon entry={entry("notes.txt")} />
        <GridIcon entry={entry("album", true)} />
      </div>,
    );
    await waitFor(() => expect(screen.getAllByTestId("grid-thumbnail")).toHaveLength(1));
    expect((screen.getByTestId("grid-thumbnail") as HTMLImageElement).src).toBe("asset://localhost/cache/cat.png.png");
    expect(container.querySelectorAll("svg")).toHaveLength(3);
    expect(mockInvoke.mock.calls.map(([, a]) => a.path)).toEqual(["/pics/cat.png", "/pics/broken.jpg"]);
  });
});
