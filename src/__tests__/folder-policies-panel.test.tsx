/**
 * Folder policies UI: opens a folder's effective policy, edits its own
 * settings (unset = inherit), saves with the server's shape, removes and
 * copies policies.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import {
  FolderPoliciesPanel,
  type FolderPolicy,
} from "@/components/server/folder-policies-panel";
import { __setAccessTokenForTests } from "@/lib/server-client";

const calls: { method: string; url: string; body: unknown }[] = [];
const effective = {
  locked: false,
  allowed_extensions: null,
  blocked_extensions: ["exe"],
  max_upload_mb: null,
  require_link_password: false,
  max_link_days: 30,
  intake_naming: "keep",
  sources: ["/srv/share"],
};

beforeEach(() => {
  calls.length = 0;
  window.__UFOP_SERVER__ = { api: "/api/v1" };
  __setAccessTokenForTests("tok");
  let own: FolderPolicy | null = null;
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string, init: RequestInit) => {
      const body = init.body ? JSON.parse(String(init.body)) : undefined;
      calls.push({ method: init.method ?? "GET", url, body });
      if (init.method === "PUT") {
        const empty = Object.values(body.policy).every((v) => v === null);
        own = empty ? null : body.policy;
      }
      if (init.method === "POST")
        return new Response(JSON.stringify({ ok: true }));
      return new Response(
        JSON.stringify({
          path: "/srv/share/inbox",
          own,
          effective: {
            ...effective,
            intake_naming: own?.intake_naming ?? "keep",
            sources: own ? ["/srv/share/inbox", "/srv/share"] : ["/srv/share"],
          },
        }),
      );
    }),
  );
});

describe("folder policies panel", () => {
  it("shows what is inherited, saves the folder's own settings, copies and removes them", async () => {
    render(<FolderPoliciesPanel />);
    fireEvent.change(screen.getByLabelText("Folder"), {
      target: { value: "/srv/share/inbox" },
    });
    fireEvent.click(screen.getByText("Open"));
    expect(await screen.findByText(/In force: open/)).toHaveTextContent(
      "except exe",
    );
    expect(screen.getByLabelText("Links expire within days")).toHaveAttribute(
      "placeholder",
      "Inherit: 30",
    );
    expect(calls[0].url).toBe(
      "/api/v1/folder-policies/effective?path=%2Fsrv%2Fshare%2Finbox",
    );

    fireEvent.change(screen.getByLabelText("Locked"), {
      target: { value: "yes" },
    });
    fireEvent.change(screen.getByLabelText("Accepted extensions"), {
      target: { value: "pdf, docx" },
    });
    fireEvent.change(screen.getByLabelText("Intake naming"), {
      target: { value: "sequence" },
    });
    fireEvent.change(screen.getByLabelText("Largest upload MB"), {
      target: { value: "50" },
    });
    fireEvent.click(screen.getByText("Save"));
    await screen.findByText("Saved.");
    expect(calls.find((c) => c.method === "PUT")!.body).toEqual({
      path: "/srv/share/inbox",
      policy: {
        locked: true,
        allowed_extensions: ["pdf", "docx"],
        blocked_extensions: null,
        max_upload_mb: 50,
        require_link_password: null,
        max_link_days: null,
        intake_naming: "sequence",
      },
    });
    expect(screen.getByText(/In force/)).toHaveTextContent("from 2 folders");

    fireEvent.change(screen.getByLabelText("Copy to folder"), {
      target: { value: "/srv/share/hr" },
    });
    fireEvent.click(screen.getByText("Copy"));
    await screen.findByText("Copied to /srv/share/hr.");
    expect(calls.find((c) => c.method === "POST")).toMatchObject({
      url: "/api/v1/folder-policies/copy",
      body: { from: "/srv/share/inbox", to: "/srv/share/hr" },
    });

    fireEvent.click(screen.getByText("Remove this folder's policy"));
    await waitFor(() =>
      expect(
        screen.queryByText("Remove this folder's policy"),
      ).not.toBeInTheDocument(),
    );
    const last = calls.filter((c) => c.method === "PUT").at(-1)!;
    expect(
      Object.values(
        (last.body as { policy: Record<string, unknown> }).policy,
      ).every((v) => v === null),
    ).toBe(true);
  });
});
