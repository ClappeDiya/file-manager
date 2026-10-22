/** Share links: provider/policy-aware options, creation, policy settings. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { ShareLinkDialog } from "@/components/share-link-dialog";
import { SharingPolicySection } from "@/components/sharing-policy-section";
import { DEFAULT_SHARING_POLICY, allowedExpiries, type SharingPolicy } from "@/stores/share-store";

let policy: SharingPolicy = DEFAULT_SHARING_POLICY;
beforeEach(() => {
  policy = { ...DEFAULT_SHARING_POLICY };
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  mockInvoke.mockReset().mockImplementation(async (cmd: string, a: { policy: SharingPolicy; options: { scope: string; expires_in_secs: number | null } }) => {
    if (cmd === "sharing_policy_get") return policy;
    if (cmd === "sharing_policy_set") return (policy = a.policy);
    if (cmd === "connector_create_share_link")
      return { url: "https://1drv.ms/x", expires_at: a.options.expires_in_secs ? "2026-10-23T10:00:00Z" : null, scope: a.options.scope, provider: "onedrive", note: null };
    return null;
  });
});

const labels = (sel: HTMLElement) => [...(sel as HTMLSelectElement).options].map((o) => o.textContent);

describe("allowedExpiries", () => {
  it("combines provider limits with the policy", () => {
    expect(allowedExpiries("s3", null).map((e) => e.label)).toEqual(["1 hour", "1 day", "7 days"]);
    expect(allowedExpiries("google_drive", null).map((e) => e.label)).toEqual(["Never"]);
    expect(allowedExpiries("google_drive", { ...DEFAULT_SHARING_POLICY, require_expiry: true })).toEqual([]);
    expect(allowedExpiries("onedrive", { ...DEFAULT_SHARING_POLICY, max_expiry_days: 7 }).map((e) => e.label)).toEqual(["1 hour", "1 day", "7 days"]);
  });
});

describe("ShareLinkDialog", () => {
  it("creates a link with the chosen options and shows it", async () => {
    render(<ShareLinkDialog protocol="onedrive" connectionId="c1" path="/Docs/a.pdf" name="a.pdf" onClose={() => {}} />);
    const scope = await screen.findByLabelText("Who can open it");
    expect(labels(scope)).toEqual(["Anyone with the link", "People in my organization"]);
    await waitFor(() => expect((screen.getByLabelText("Expires after") as HTMLSelectElement).value).toBe("86400"));
    fireEvent.change(screen.getByLabelText("Link password"), { target: { value: "pw" } });
    fireEvent.click(screen.getByLabelText("Allow editing"));
    fireEvent.click(screen.getByRole("button", { name: /Create link/ }));
    expect(((await screen.findByLabelText("Link")) as HTMLInputElement).value).toBe("https://1drv.ms/x");
    expect(mockInvoke).toHaveBeenCalledWith("connector_create_share_link", {
      protocol: "onedrive", connectionId: "c1", path: "/Docs/a.pdf", options: { expires_in_secs: 86400, password: "pw", scope: "anyone", allow_edit: true },
    });
  });

  it("hides options the provider lacks", async () => {
    render(<ShareLinkDialog protocol="s3" connectionId={null} path="b/k" name="k" onClose={() => {}} />);
    await screen.findByLabelText("Expires after");
    expect(screen.queryByLabelText("Who can open it")).toBeNull();
    expect(screen.queryByLabelText("Link password")).toBeNull();
    expect(screen.queryByLabelText("Allow editing")).toBeNull();
  });

  it("explains when the policy blocks sharing", async () => {
    policy = { ...DEFAULT_SHARING_POLICY, allow_anyone: false };
    render(<ShareLinkDialog protocol="s3" connectionId={null} path="b/k" name="k" onClose={() => {}} />);
    expect((await screen.findByRole("alert")).textContent).toContain("only allows organization links");
    expect((screen.getByRole("button", { name: /Create link/ }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("requires a password when the policy does", async () => {
    policy = { ...DEFAULT_SHARING_POLICY, require_password: true };
    render(<ShareLinkDialog protocol="dropbox" connectionId={null} path="/a" name="a" onClose={() => {}} />);
    await screen.findByText(/required by policy/);
    const create = screen.getByRole("button", { name: /Create link/ }) as HTMLButtonElement;
    expect(create.disabled).toBe(true);
    fireEvent.change(screen.getByLabelText("Link password"), { target: { value: "x" } });
    expect(create.disabled).toBe(false);
  });
});

describe("SharingPolicySection", () => {
  it("saves changes", async () => {
    render(<SharingPolicySection />);
    fireEvent.click(await screen.findByLabelText(/Links must expire/));
    await screen.findByText("Saved");
    fireEvent.change(screen.getByLabelText("Longest allowed expiry (days)"), { target: { value: "14" } });
    await waitFor(() => expect(policy).toMatchObject({ require_expiry: true, max_expiry_days: 14 }));
  });
});
