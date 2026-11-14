/**
 * ufop-server share links UI: talks to /api/v1/shares with the session
 * token, shows a new link once, lists links with status, shows activity and
 * turns links off; explains when sharing is off.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import {
  ShareLinksPanel,
  type ServerShare,
} from "@/components/server/share-links-panel";
import { __setAccessTokenForTests } from "@/lib/server-client";

const share = (over: Partial<ServerShare> = {}): ServerShare => ({
  id: "s1",
  name: "Q3 docs",
  path: "/srv/share/q3",
  kind: "download",
  token_prefix: "AbC123",
  has_password: true,
  expires_at: null,
  max_downloads: 5,
  downloads: 5,
  max_upload_bytes: null,
  created_at: "2026-10-01T00:00:00Z",
  revoked_at: null,
  last_access_at: null,
  ...over,
});

let calls: {
  method: string;
  url: string;
  body: unknown;
  auth: string | null;
}[] = [];

function mockServer(
  handler: (method: string, url: string, body: unknown) => unknown,
) {
  calls = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (url: string, init: RequestInit) => {
      const body = init.body ? JSON.parse(String(init.body)) : undefined;
      calls.push({
        method: init.method ?? "GET",
        url,
        body,
        auth: new Headers(init.headers).get("Authorization"),
      });
      const out = handler(init.method ?? "GET", url, body);
      if (out instanceof Response) return out;
      return new Response(JSON.stringify(out), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    }),
  );
}

beforeEach(() => {
  window.__UFOP_SERVER__ = { api: "/api/v1" };
  __setAccessTokenForTests("tok");
});

describe("share links panel", () => {
  it("creates a link, shows it once and lists links with status", async () => {
    let list = [share()];
    mockServer((method, url, body) => {
      if (method === "GET" && url === "/api/v1/shares")
        return { shares: list, roots: ["/srv/share"] };
      if (method === "POST" && url === "/api/v1/shares") {
        list = [
          share({
            id: "s2",
            name: (body as { name: string }).name,
            kind: "upload",
            has_password: false,
            max_downloads: null,
            downloads: 0,
          }),
          ...list,
        ];
        return {
          share: list[0],
          token: "secret-token",
          url: "/s/secret-token",
        };
      }
      return {};
    });
    render(<ShareLinksPanel />);
    const table = await screen.findByLabelText("Share links");
    expect(within(table).getByText("Used up")).toBeInTheDocument();
    expect(
      screen.getByText(/Files and folders under \/srv\/share/),
    ).toBeInTheDocument();

    fireEvent.change(screen.getByLabelText("Path"), {
      target: { value: "/srv/share/inbox" },
    });
    fireEvent.change(screen.getByLabelText("Link kind"), {
      target: { value: "upload" },
    });
    expect(screen.queryByLabelText("Download limit")).not.toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Largest upload"), {
      target: { value: "50" },
    });
    fireEvent.change(screen.getByLabelText("Expires after days"), {
      target: { value: "7" },
    });
    fireEvent.click(screen.getByText("Create link"));
    expect(await screen.findByLabelText("New link")).toHaveValue(
      `${window.location.origin}/s/secret-token`,
    );
    const post = calls.find((c) => c.method === "POST")!;
    expect(post.auth).toBe("Bearer tok");
    expect(post.body).toEqual({
      name: "inbox",
      path: "/srv/share/inbox",
      kind: "upload",
      password: null,
      expires_in_days: 7,
      max_downloads: null,
      max_upload_mb: 50,
    });
    await waitFor(() =>
      expect(
        within(screen.getByLabelText("Share links")).getByText("inbox"),
      ).toBeInTheDocument(),
    );
  });

  it("shows activity and turns a link off", async () => {
    let revoked = false;
    mockServer((method, url) => {
      if (method === "GET" && url === "/api/v1/shares")
        return {
          shares: [
            share({ revoked_at: revoked ? "2026-10-02T00:00:00Z" : null }),
          ],
          roots: ["/srv/share"],
        };
      if (url === "/api/v1/shares/s1/events?limit=100")
        return {
          events: [
            {
              at: "2026-10-01T10:00:00Z",
              ip: "203.0.113.9",
              action: "download",
              detail: "report.pdf (1.0 KB)",
            },
          ],
        };
      if (method === "DELETE") {
        revoked = true;
        return { ok: true };
      }
      return {};
    });
    render(<ShareLinksPanel />);
    fireEvent.click(await screen.findByText("Activity"));
    expect(await screen.findByLabelText("Link activity")).toHaveTextContent(
      "report.pdf (1.0 KB)",
    );
    fireEvent.click(screen.getByLabelText("Turn off Q3 docs"));
    await waitFor(() =>
      expect(
        within(screen.getByLabelText("Share links")).getByText("Off"),
      ).toBeInTheDocument(),
    );
    expect(
      calls.some((c) => c.method === "DELETE" && c.url === "/api/v1/shares/s1"),
    ).toBe(true);
  });

  it("explains when sharing is off, and surfaces server errors", async () => {
    mockServer(() => ({ shares: [], roots: [] }));
    const { unmount } = render(<ShareLinksPanel />);
    expect(await screen.findByTestId("share-links-off")).toHaveTextContent(
      "--share-root",
    );
    unmount();
    mockServer((method) =>
      method === "POST"
        ? new Response(
            JSON.stringify({
              error_type: "validation",
              message: "Link passwords need at least 8 characters",
              advice: "",
            }),
            { status: 400 },
          )
        : { shares: [], roots: ["/srv/share"] },
    );
    render(<ShareLinksPanel />);
    fireEvent.change(await screen.findByLabelText("Path"), {
      target: { value: "/srv/share/x" },
    });
    fireEvent.click(screen.getByText("Create link"));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "at least 8 characters",
    );
  });
});
