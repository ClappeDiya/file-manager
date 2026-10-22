/** Server web UI: file gateways — listing, browsing, download and upload requests. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { GatewaysPanel } from "@/components/server/gateways-panel";
import { __setAccessTokenForTests } from "@/lib/server-client";

const fetchMock = vi.fn();
const json = (v: unknown, status = 200) => new Response(JSON.stringify(v), { status, headers: { "Content-Type": "application/json" } });
const node = { node: "n1", host: "nas-1", ip: "10.0.0.5", roots: ["projects"], writable: true, version: "1", last_seen: "2026-01-01T00:00:00Z", online: true, jobs: 3 };

beforeEach(() => {
  fetchMock.mockReset();
  vi.stubGlobal("fetch", fetchMock);
  __setAccessTokenForTests("tok");
  URL.createObjectURL = vi.fn(() => "blob:x");
  URL.revokeObjectURL = vi.fn();
});

describe("gateways panel", () => {
  it("shows gateways and nodes, browses, downloads and uploads", async () => {
    fetchMock.mockImplementation(async (url: string, init: RequestInit) => {
      if (url.endsWith("/api/v1/gateways")) return json({ gateways: [{ name: "office", online: true, online_nodes: 1, writable: true, queued: 0, nodes: [node] }] });
      if (url.includes("/files?path=%2F") && !url.includes("projects")) return json({ path: "/", entries: [{ name: "projects", is_dir: true, size: 0, modified: null }] });
      if (url.includes("/files?path=%2Fprojects")) return json({ path: "/projects", entries: [{ name: "a b.txt", is_dir: false, size: 2048, modified: null }] });
      if (url.includes("/download?")) return new Response("data");
      if (url.includes("/upload?") && init.method === "PUT") return json({ path: "/projects/up.txt", bytes: 2 });
      return json({ message: "unexpected" }, 500);
    });
    render(<GatewaysPanel />);
    expect(await screen.findByText("office")).toBeInTheDocument();
    expect(screen.getByText(/1\/1 node\(s\) online · uploads allowed/)).toBeInTheDocument();
    expect(screen.getByText(/nas-1 \(10.0.0.5\) · projects · online · 3 job/)).toBeInTheDocument();

    fireEvent.click(screen.getByText("Browse"));
    fireEvent.click(await screen.findByText("projects/"));
    fireEvent.click(await screen.findByText("a b.txt"));
    await waitFor(() => expect(fetchMock.mock.calls.some(([u]) => String(u).endsWith("/gateways/office/download?path=%2Fprojects%2Fa%20b.txt"))).toBe(true));
    const auth = (fetchMock.mock.calls.find(([u]) => String(u).includes("/download?"))![1] as RequestInit).headers as Record<string, string>;
    expect(auth.Authorization).toBe("Bearer tok");

    const file = new File(["hi"], "up.txt");
    fireEvent.change(screen.getByLabelText("Upload files"), { target: { files: [file] } });
    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent("Uploaded 1 file(s)."));
    const put = fetchMock.mock.calls.find(([, i]) => (i as RequestInit).method === "PUT")!;
    expect(String(put[0])).toContain("/gateways/office/upload?path=%2Fprojects%2Fup.txt");
    expect((put[1] as RequestInit).body).toBe(file);
  });

  it("shows the server's error and an empty state", async () => {
    fetchMock.mockResolvedValueOnce(json({ gateways: [] }));
    render(<GatewaysPanel />);
    expect(await screen.findByText("No gateway has connected yet.")).toBeInTheDocument();
    fetchMock.mockResolvedValueOnce(json({ message: "No node of gateway office is online", advice: "" }, 503));
    fireEvent.click(screen.getByText("Refresh"));
    expect(await screen.findByRole("alert")).toHaveTextContent("No node of gateway office is online");
  });
});
