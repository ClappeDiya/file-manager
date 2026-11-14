/** Convert dialog: images vs documents, the spec sent to Rust, results, and the context menu entry. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { ConvertDialog } from "@/components/convert-dialog";
import { getFileContextMenuItems } from "@/components/context-menu";
import { convertKind, useConvertDialog } from "@/stores/convert-store";

let office = true;
beforeEach(() => {
  office = true;
  useConvertDialog.getState().close();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  mockInvoke.mockReset().mockImplementation(async (cmd: string, a: Record<string, unknown>) => {
    switch (cmd) {
      case "convert_capabilities":
        return { image_formats: ["jpeg", "png", "webp"], document_formats: ["pdf", "odt"], office };
      case "convert_images":
        return (a.paths as string[]).map((p) => ({ input: p, output: p.replace(/\.\w+$/, ".webp"), error: null }));
      case "convert_documents":
        return [{ input: "/d/memo.docx", output: null, error: "LibreOffice could not convert /d/memo.docx to odt" }];
      default:
        return null;
    }
  });
});

describe("convertKind", () => {
  it("accepts all-image or all-document selections only", () => {
    expect(convertKind(["/a.PNG", "/b.jpeg"])).toBe("image");
    expect(convertKind(["/a.docx", "/b.txt"])).toBe("document");
    expect(convertKind(["/a.png", "/b.docx"])).toBeNull();
    expect(convertKind(["/a.exe"])).toBeNull();
    expect(convertKind([])).toBeNull();
  });
});

describe("ConvertDialog", () => {
  it("resizes and converts images with the chosen spec", async () => {
    render(<ConvertDialog />);
    act(() => useConvertDialog.getState().open(["/d/a.png", "/d/b.jpg"]));
    expect(screen.getByText("Convert 2 images")).toBeTruthy();
    await screen.findByRole("option", { name: "WEBP" });
    fireEvent.change(screen.getByLabelText("Format"), { target: { value: "webp" } });
    expect(screen.queryByLabelText(/Quality/)).toBeNull();
    fireEvent.change(screen.getByLabelText("Largest height (px)"), { target: { value: "" } });
    fireEvent.click(screen.getByRole("button", { name: "Convert" }));
    expect(await screen.findByText("→ a.webp")).toBeTruthy();
    expect(mockInvoke).toHaveBeenCalledWith("convert_images", { paths: ["/d/a.png", "/d/b.jpg"], spec: { format: "webp", max_width: 1920, max_height: null, quality: 85 }, destDir: null });
  });

  it("converts documents and shows per-file errors", async () => {
    render(<ConvertDialog />);
    act(() => useConvertDialog.getState().open(["/d/memo.docx"]));
    await screen.findByRole("option", { name: "ODT" });
    fireEvent.change(screen.getByLabelText("Convert to"), { target: { value: "odt" } });
    fireEvent.change(screen.getByLabelText("Save into folder"), { target: { value: "/out" } });
    fireEvent.click(screen.getByRole("button", { name: "Convert" }));
    expect(await screen.findByText(/could not convert/)).toBeTruthy();
    expect(mockInvoke).toHaveBeenCalledWith("convert_documents", { paths: ["/d/memo.docx"], to: "odt", destDir: "/out" });
  });

  it("explains when LibreOffice is missing", async () => {
    office = false;
    render(<ConvertDialog />);
    act(() => useConvertDialog.getState().open(["/d/memo.docx"]));
    expect((await screen.findByRole("alert")).textContent).toContain("LibreOffice was not found");
    await waitFor(() => expect((screen.getByRole("button", { name: "Convert" }) as HTMLButtonElement).disabled).toBe(true));
  });
});

describe("context menu", () => {
  it("offers Convert only when the caller allows it", () => {
    const base = { hasSelection: true, isDirectory: false, selectionCount: 2, onCopy() {}, onCut() {}, onPaste() {}, onDelete() {}, onRename() {}, onDuplicate() {} };
    const items = (o: object) => getFileContextMenuItems({ ...base, ...o } as Parameters<typeof getFileContextMenuItems>[0]);
    expect(items({ onConvert: () => {} }).find((i) => i.id === "convert")?.label).toBe("Convert 2 Files…");
    expect(items({}).some((i) => i.id === "convert")).toBe(false);
    expect(items({ onConvert: () => {}, isDirectory: true }).some((i) => i.id === "convert")).toBe(false);
  });
});
