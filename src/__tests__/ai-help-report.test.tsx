/** AI panel Help (docs-backed answers with sources) and Report (activity) tabs. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { HelpTab, ReportTab } from "@/components/ai-help-report";

let model = false;
beforeEach(() => {
  model = false;
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  mockInvoke.mockReset().mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "ai_help":
        return {
          answer: model ? "Right-click and choose Re-encrypt." : "From the User Guide — **OpenPGP (GPG)**:\n\nRe-encrypt changes who can open a file.",
          generated_by: model ? "model" : "docs",
          sources: [
            { doc: "User Guide", section: "OpenPGP (GPG)", excerpt: "Re-encrypt changes who can open a file.", score: 9 },
            { doc: "Troubleshooting", section: "GPG", excerpt: "Install GnuPG.", score: 3 },
          ],
        };
      case "ai_activity_report":
        return { days: 7, from: "", to: "", markdown: "# Activity report — last 7 days\n\n| Upload | 3 | 2 |", summary: model ? "Upload is failing." : null, runs: 5, failed_runs: 2, operations: 3, failed_operations: 1 };
      default:
        return null;
    }
  });
});

describe("HelpTab", () => {
  it("answers from the docs and lists expandable sources", async () => {
    render(<HelpTab />);
    fireEvent.change(screen.getByLabelText("Question"), { target: { value: "re-encrypt for another key" } });
    fireEvent.click(screen.getByRole("button", { name: "Ask" }));
    expect((await screen.findByLabelText("Answer")).textContent).toContain("From the User Guide");
    expect(screen.getByText(/no local model is running/)).toBeTruthy();
    expect(mockInvoke).toHaveBeenCalledWith("ai_help", { question: "re-encrypt for another key" });
    const src = screen.getByRole("button", { name: /Troubleshooting — GPG/ });
    fireEvent.click(src);
    expect(src.getAttribute("aria-expanded")).toBe("true");
    expect(screen.getByText("Install GnuPG.")).toBeTruthy();
  });

  it("says when the local model wrote the answer", async () => {
    model = true;
    render(<HelpTab />);
    fireEvent.change(screen.getByLabelText("Question"), { target: { value: "re-encrypt" } });
    fireEvent.submit(screen.getByLabelText("Question").closest("form")!);
    expect(await screen.findByText("Right-click and choose Re-encrypt.")).toBeTruthy();
    expect(screen.getByText(/Written by your local AI model/)).toBeTruthy();
  });
});

describe("ReportTab", () => {
  it("creates a report for the chosen period and saves it with the summary", async () => {
    model = true;
    render(<ReportTab />);
    fireEvent.change(screen.getByLabelText("Period"), { target: { value: "30" } });
    fireEvent.click(screen.getByRole("button", { name: /Create report/ }));
    expect((await screen.findByLabelText("Report")).textContent).toContain("Activity report");
    expect(screen.getByLabelText("Summary").textContent).toBe("Upload is failing.");
    expect(screen.getByLabelText("Totals").textContent).toContain("2 failed");
    expect(mockInvoke).toHaveBeenCalledWith("ai_activity_report", { days: 30 });
    fireEvent.change(screen.getByLabelText("Save to"), { target: { value: "/tmp/r.md" } });
    fireEvent.click(screen.getByRole("button", { name: /Save/ }));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("write_text_file_full", { path: "/tmp/r.md", content: "> Upload is failing.\n\n# Activity report — last 7 days\n\n| Upload | 3 | 2 |" }));
    expect(await screen.findByText("Saved to /tmp/r.md.")).toBeTruthy();
  });
});
