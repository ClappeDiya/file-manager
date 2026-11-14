// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest";
import { afterEach, expect, it, vi } from "vitest";
import {
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import ApprovalsPage from "../../app/approvals/page";
vi.mock("@/lib/stores/auth-store", () => ({ useAuthStore: () => ({ user: { id: "reviewer", role: "org_owner" } }) }));
vi.mock("next/navigation", () => ({ usePathname: () => "/approvals" }));
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});
it.each(["Approve", "Deny"])(
  "retains a pending request and its comment when %s fails",
  async (action) => {
    const approval = {
      id: "request",
      operation: "Copy reviewed files",
      trigger: "external_dest_transfer",
      source: "/source",
      destination: "/backup",
      fileCount: 2,
      totalSize: 14,
      reason: "Review required",
      triggeringPolicy: "External copies",
      state: "pending",
      requestedBy: "requester",
      requestedByName: "Requester",
      requestedAt: new Date().toISOString(),
      expiresAt: new Date(Date.now() + 3600000).toISOString(),
      organizationId: "test-org",
    };
    vi.stubGlobal(
      "fetch",
      vi.fn(async (_url: string, options?: RequestInit) =>
        options?.method === "PATCH"
          ? { ok: false }
          : { ok: true, json: async () => ({ approvals: [approval] }) },
      ),
    );
    render(<ApprovalsPage />);
    fireEvent.click(await screen.findByRole("button", { name: action }));
    const dialog = screen.getByRole("dialog");
    const comment = within(dialog).getByRole("textbox");
    fireEvent.change(comment, { target: { value: "Retain this review" } });
    fireEvent.click(within(dialog).getByRole("button", { name: action }));
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "could not be confirmed",
    );
    expect(comment).toHaveValue("Retain this review");
    expect(within(dialog).getByRole("button", { name: action })).toBeVisible();
    expect(screen.getAllByText("Pending Review").length).toBeGreaterThan(0);
  },
);
