/** Copy to several folders: candidates, per-destination results, refresh. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { CopyToManyDialog } from "@/components/copy-to-many-dialog";
import { destinationCandidates, openCopyToMany, useCopyToManyStore } from "@/stores/copy-to-many-store";
import { useFileManagerStore } from "@/stores/file-manager-store";

beforeEach(() => {
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  useCopyToManyStore.getState().close();
  useFileManagerStore.setState({ favorites: [{ id: "1", name: "Docs", path: "/docs" }, { id: "2", name: "Here", path: "/here" }] });
});

describe("copy to several folders", () => {
  it("offers the other pane and favorites, never the current folder", () => {
    expect(destinationCandidates("/here", "/other", ["/docs", "/here", "/other", ""])).toEqual(["/other", "/docs"]);
  });

  it("copies to the chosen folders and reports each outcome", async () => {
    mockInvoke.mockImplementation(async (cmd: string, args: { destDirs: string[] }) =>
      cmd === "copy_files_to_many"
        ? args.destDirs.map((d) => (d === "/docs" ? { dest_dir: d, result: null, error: "Destination directory does not exist: /docs" } : { dest_dir: d, result: { dest_paths: [`${d}/a`], undo_id: "u" }, error: null }))
        : null,
    );
    render(<CopyToManyDialog />);
    act(() => openCopyToMany(["/here/a"], "/here", "/other"));
    expect(screen.getByRole("dialog", { name: "Copy 1 item to several folders" })).toBeTruthy();
    const copy = screen.getByRole("button", { name: /Copy to 0 folders/ }) as HTMLButtonElement;
    expect(copy.disabled).toBe(true);
    fireEvent.click(screen.getByLabelText("/other"));
    fireEvent.click(screen.getByLabelText("/docs"));
    fireEvent.click(screen.getByRole("button", { name: "Copy to 2 folders" }));
    await waitFor(() => expect(screen.getByText("Copied")).toBeTruthy());
    expect(mockInvoke).toHaveBeenCalledWith("copy_files_to_many", { sourcePaths: ["/here/a"], destDirs: ["/other", "/docs"] });
    expect(screen.getByRole("alert").textContent).toContain("/docs: Destination directory does not exist");
    fireEvent.click(screen.getByText("Done"));
    expect(screen.queryByRole("dialog")).toBeNull();
  });
});
