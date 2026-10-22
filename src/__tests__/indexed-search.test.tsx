/** Indexed search UI: results, index-this-folder offer, reveal, simple-mode view. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { IndexedSearchResults } from "@/components/indexed-search-results";
import { SimpleSearchView } from "@/components/simple-mode/simple-search-view";
import { snippetParts, type IndexRoot, type SearchHit } from "@/stores/search-store";
import { useFileManagerStore } from "@/stores/file-manager-store";

const ROOT: IndexRoot = { path: "/home/me", content: true, indexed_at: "2026-10-01T00:00:00Z", files: 10 };
const HIT: SearchHit = { path: "/home/me/docs/report.txt", name: "report.txt", is_dir: false, size: 2048, modified: 0, snippet: "grew in the [third] [quarter]…", source: null };
let covered: IndexRoot | null = ROOT;

beforeEach(() => {
  covered = ROOT;
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  mockInvoke.mockReset().mockImplementation(async (cmd: string) => {
    if (cmd === "search_files_query") return { hits: covered ? [HIT] : [], root: covered, indexing: false };
    if (cmd === "search_index_add") {
      covered = ROOT;
      return { roots: [ROOT], jobs: [{ root: "/home/me", scanned: 0, running: false, last: null, error: null }] };
    }
    if (cmd === "search_index_roots") return { roots: [ROOT], jobs: [] };
    return null;
  });
});

describe("snippetParts", () => {
  it("splits matched terms", () => {
    expect(snippetParts("a [b] c [d]")).toEqual([
      { text: "a ", hit: false },
      { text: "b", hit: true },
      { text: " c ", hit: false },
      { text: "d", hit: true },
    ]);
  });
});

describe("IndexedSearchResults", () => {
  it("lists content hits with highlighted text and opens them", async () => {
    const onOpen = vi.fn();
    render(<IndexedSearchResults text="third quarter" root="/home/me" content onOpen={onOpen} />);
    const hit = await screen.findByRole("button", { name: /report\.txt/ });
    expect(hit.textContent).toContain("docs");
    expect(screen.getAllByText("third")[0].tagName).toBe("MARK");
    expect(mockInvoke).toHaveBeenCalledWith("search_files_query", { text: "third quarter", root: "/home/me", content: true, limit: 200 });
    fireEvent.click(hit);
    expect(onOpen).toHaveBeenCalledWith("/home/me/docs/report.txt", false);
  });

  it("offers to index a folder that isn't, then searches it", async () => {
    covered = null;
    render(<IndexedSearchResults text="report" root="/home/me/" content={false} onOpen={() => {}} />);
    fireEvent.click(await screen.findByText("Index this folder"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("search_index_add", { path: "/home/me/", content: true }));
    expect(await screen.findByRole("button", { name: /report\.txt/ })).toBeTruthy();
  });
});

describe("revealPath", () => {
  it("opens a file's folder and selects it", () => {
    const s = useFileManagerStore.getState();
    s.revealPath(0, "/home/me/docs/report.txt", false);
    const st = useFileManagerStore.getState();
    const pane = st.panes[0];
    expect(st.getActivePath(0)).toBe("/home/me/docs");
    expect(st.selectedPaths[pane.id]).toEqual(["/home/me/docs/report.txt"]);
    s.revealPath(0, "/home/me/docs", true);
    expect(useFileManagerStore.getState().getActivePath(0)).toBe("/home/me/docs");
  });
});

describe("server hits", () => {
  it("are marked with their protocol and don't open in a local pane", async () => {
    mockInvoke.mockImplementation(async (cmd: string) =>
      cmd === "search_files_query"
        ? { hits: [{ ...HIT, path: "/srv/report.txt", source: "sftp", snippet: null }], root: ROOT, indexing: false }
        : { roots: [ROOT, { path: "remote:sftp", content: false, indexed_at: null, files: 3 }], jobs: [] },
    );
    const onOpen = vi.fn();
    render(<IndexedSearchResults text="report" root="/home/me" content={false} onOpen={onOpen} />);
    const hit = (await screen.findByRole("button", { name: /report\.txt/ })) as HTMLButtonElement;
    expect(hit.textContent).toContain("SFTP");
    expect(hit.disabled).toBe(true);
    render(<SimpleSearchView onOpen={onOpen} />);
    expect(await screen.findByText("Folders browsed on SFTP")).toBeTruthy();
  });
});

describe("SimpleSearchView", () => {
  it("searches every indexed folder and lists them", async () => {
    const onOpen = vi.fn();
    render(<SimpleSearchView onOpen={onOpen} />);
    expect(await screen.findByText("/home/me")).toBeTruthy();
    fireEvent.change(screen.getByLabelText("Search files"), { target: { value: "report" } });
    fireEvent.click(await screen.findByRole("button", { name: /report\.txt/ }));
    expect(onOpen).toHaveBeenCalledWith("/home/me/docs/report.txt", false);
    expect(mockInvoke).toHaveBeenCalledWith("search_files_query", { text: "report", root: "", content: false, limit: 200 });
  });
});
