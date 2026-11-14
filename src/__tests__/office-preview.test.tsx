/**
 * Office preview: text first, then the rendered page (PDF or image) from
 * `preview_office_render`; without a renderer the reason and the text stay,
 * and the file can be opened in its app.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...a: unknown[]) => mockInvoke(...a),
  convertFileSrc: (p: string) => `asset://localhost/${encodeURIComponent(p)}`,
}));

import { OfficePreview } from "@/components/office-preview";

beforeEach(() => {
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

describe("office preview", () => {
  it("shows the rendered PDF with a text tab", async () => {
    mockInvoke.mockResolvedValue({
      kind: "pdf",
      path: "/cache/abc.pdf",
      renderer: "libreoffice",
      message: null,
    });
    render(
      <OfficePreview
        path="/d/report.docx"
        fileName="report.docx"
        text={"Quarterly Report\nRevenue"}
      />,
    );
    expect(screen.getByText("Rendering preview…")).toBeInTheDocument();
    const frame = await screen.findByTitle("report.docx preview");
    expect(frame).toHaveAttribute(
      "src",
      `asset://localhost/${encodeURIComponent("/cache/abc.pdf")}`,
    );
    expect(mockInvoke).toHaveBeenCalledWith("preview_office_render", {
      path: "/d/report.docx",
    });
    fireEvent.click(screen.getByRole("tab", { name: "Text" }));
    expect(screen.getByLabelText("Document text")).toHaveTextContent(
      "Quarterly Report Revenue",
    );
    expect(screen.queryByTitle("report.docx preview")).not.toBeInTheDocument();
  });

  it("shows a Quick Look image", async () => {
    mockInvoke.mockResolvedValue({
      kind: "image",
      path: "/cache/abc.png",
      renderer: "quicklook",
      message: null,
    });
    render(
      <OfficePreview path="/d/deck.pptx" fileName="deck.pptx" text={null} />,
    );
    expect(await screen.findByAltText("deck.pptx")).toBeInTheDocument();
    expect(screen.queryByRole("tablist")).not.toBeInTheDocument();
  });

  it("falls back to the text, explains why, and opens the file in its app", async () => {
    mockInvoke.mockImplementation(async (cmd: string) =>
      cmd === "preview_office_render"
        ? {
            kind: "none",
            path: null,
            renderer: null,
            message: "No document renderer is installed.",
          }
        : undefined,
    );
    render(
      <OfficePreview
        path="/d/budget.xlsx"
        fileName="budget.xlsx"
        text={"Budget\nItem\tCost"}
      />,
    );
    expect(
      await screen.findByText("No document renderer is installed."),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Document text")).toHaveTextContent("Item");
    fireEvent.click(screen.getByText("Open in default app"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("open_file_with_default", {
        path: "/d/budget.xlsx",
      }),
    );
  });

  it("says so when a legacy file has neither text nor a renderer, and shows errors", async () => {
    mockInvoke.mockRejectedValueOnce(new Error("File not found"));
    const { rerender } = render(
      <OfficePreview path="/d/gone.doc" fileName="gone.doc" text={null} />,
    );
    expect(await screen.findByText(/File not found/)).toBeInTheDocument();
    mockInvoke.mockResolvedValue({
      kind: "none",
      path: null,
      renderer: null,
      message: "No document renderer is installed.",
    });
    rerender(
      <OfficePreview path="/d/old.doc" fileName="old.doc" text={null} />,
    );
    expect(
      await screen.findByText("No text preview for this format."),
    ).toBeInTheDocument();
  });
});
