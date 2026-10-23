/**
 * Text editor drafts and autosave: edits are kept as a draft, a leftover
 * draft is offered back on open, saving clears it, and autosave writes
 * after a pause but never retries a refused save in a loop.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { AUTOSAVE_MS, DRAFT_DEBOUNCE_MS, TextEditorModal, type EditorDraft } from "@/components/text-editor-modal";
import { useUIStore } from "@/stores/ui-store";

let disk = "on disk";
let mtime = "2026-01-01T00:00:00Z";
let draft: EditorDraft | null = null;
let writeFails = false;
const calls = (cmd: string) => mockInvoke.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a);

async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

beforeEach(() => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  disk = "on disk";
  mtime = "2026-01-01T00:00:00Z";
  draft = null;
  writeFails = false;
  useUIStore.setState({ editorAutosave: false });
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  mockInvoke.mockReset().mockImplementation(async (cmd: string, a: Record<string, string>) => {
    switch (cmd) {
      case "read_text_file_full":
        return disk;
      case "get_file_metadata":
        return { modified: mtime };
      case "editor_draft_load":
        return draft;
      case "editor_draft_save":
        draft = { path: a.path, content: a.content, base_modified: a.baseModified, saved_at: "2026-01-02T00:00:00Z" };
        return draft;
      case "editor_draft_discard":
        draft = null;
        return null;
      case "write_text_file_full":
        if (writeFails) throw "Disk full";
        disk = a.content;
        return null;
      default:
        return null;
    }
  });
});
afterEach(() => vi.useRealTimers());

const open = async () => {
  render(<TextEditorModal filePath="/n.txt" fileSize={10} onClose={() => {}} />);
  await flush();
  await flush();
};
const type = (text: string) => fireEvent.change(screen.getByRole("textbox"), { target: { value: text } });

describe("editor drafts", () => {
  it("keeps a draft of unsaved edits and clears it on save", async () => {
    await open();
    type("edited");
    await act(async () => vi.advanceTimersByTime(DRAFT_DEBOUNCE_MS + 10));
    expect(draft).toMatchObject({ path: "/n.txt", content: "edited", base_modified: mtime });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await flush();
    await flush();
    expect(disk).toBe("edited");
    expect(draft).toBeNull();
  });

  it("offers a leftover draft and restores it as unsaved", async () => {
    draft = { path: "/n.txt", content: "from crash", base_modified: "2025-12-31T00:00:00Z", saved_at: "2026-01-02T00:00:00Z" };
    await open();
    const banner = screen.getByRole("status", { name: "Recovered draft" });
    expect(banner.textContent).toContain("changed on disk since then");
    fireEvent.click(screen.getByText("Restore"));
    await flush();
    expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe("from crash");
    expect(screen.getByText("Modified")).toBeTruthy();
    expect(disk).toBe("on disk");
  });

  it("discarding the recovered draft deletes it", async () => {
    draft = { path: "/n.txt", content: "old", base_modified: mtime, saved_at: "2026-01-02T00:00:00Z" };
    await open();
    fireEvent.click(screen.getByText("Discard"));
    await flush();
    expect(draft).toBeNull();
    expect((screen.getByRole("textbox") as HTMLTextAreaElement).value).toBe("on disk");
  });

  it("drops a draft identical to the file without asking", async () => {
    draft = { path: "/n.txt", content: "on disk", base_modified: mtime, saved_at: "x" };
    await open();
    expect(screen.queryByRole("status", { name: "Recovered draft" })).toBeNull();
    expect(draft).toBeNull();
  });
});

describe("autosave", () => {
  it("saves after a pause when enabled", async () => {
    await open();
    fireEvent.click(screen.getByLabelText("Autosave"));
    expect(useUIStore.getState().editorAutosave).toBe(true);
    type("auto");
    await act(async () => vi.advanceTimersByTime(AUTOSAVE_MS + 10));
    await flush();
    await flush();
    expect(disk).toBe("auto");
  });

  it("does not retry a failed save until the text changes", async () => {
    useUIStore.setState({ editorAutosave: true });
    writeFails = true;
    await open();
    type("a");
    for (let i = 0; i < 4; i++) {
      await act(async () => vi.advanceTimersByTime(AUTOSAVE_MS + 10));
      await flush();
    }
    expect(calls("write_text_file_full")).toHaveLength(1);
    expect(screen.getByText(/Save failed: Disk full/)).toBeTruthy();
    type("ab");
    await act(async () => vi.advanceTimersByTime(AUTOSAVE_MS + 10));
    await flush();
    expect(calls("write_text_file_full")).toHaveLength(2);
  });
});
