/** Nearby computers panel: sends the chosen files, shows real transfer
 *  progress, answers requests (Downloads or "Save here"), and polls the
 *  backend at a steady rate instead of on every render. */
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { PeerSidebar, type PeerDeviceData, type PeerTransferProgress, type PeerTransferRequestData } from "@/components/peer-sidebar";
import { getFileContextMenuItems } from "@/components/context-menu";
import { getDefaultCommands } from "@/components/command-palette";

const peer: PeerDeviceData = {
  id: "p1",
  hostname: "studio-mac",
  os: "macos",
  addresses: ["192.168.1.20"],
  port: 43210,
  app_version: "1.0.0",
  status: "online",
  trust: "untrusted",
  discovered_at: "2026-01-01T00:00:00Z",
  last_seen: "2026-01-01T00:00:00Z",
  display_name: null,
  tls_fingerprint: "ab".repeat(32),
};

const transfer: PeerTransferProgress = {
  request_id: "r1",
  peer_id: "p1",
  peer_name: "studio-mac",
  direction: "outgoing",
  status: "transferring",
  transferred_bytes: 512,
  total_bytes: 1024,
  current_file: "a.bin",
  files_completed: 1,
  files_total: 2,
  speed_bps: 2048,
  eta_seconds: 1,
  save_path: null,
  error: null,
};

const request: PeerTransferRequestData = {
  request_id: "r9",
  sender_id: "p1",
  sender_hostname: "studio-mac",
  sender_os: "macos",
  files: [
    { relative_path: "album", size: 0, is_dir: true, modified: null },
    { relative_path: "album/a.jpg", size: 10, is_dir: false, modified: null },
  ],
  total_bytes: 10,
  file_count: 2,
  requested_at: "2026-01-01T00:00:00Z",
};

function backend(overrides: Record<string, unknown> = {}) {
  mockInvoke.mockImplementation(async (cmd: string) => {
    if (cmd in overrides) {
      const v = overrides[cmd];
      if (v instanceof Error) throw v;
      return v;
    }
    switch (cmd) {
      case "peer_list_peers": return [peer];
      case "peer_list_saved": return [];
      case "peer_list_pending_requests": return [];
      case "peer_list_transfers": return [];
      case "peer_is_discovering": return false;
      case "peer_local_info": return { peer_id: "me", fingerprint: "0123456789abcdef".repeat(4), port: 43210, discovering: false };
      default: return undefined;
    }
  });
}

beforeAll(() => {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});
afterEach(() => {
  vi.useRealTimers();
  mockInvoke.mockReset();
});

describe("PeerSidebar", () => {
  it("polls at the refresh interval, not on every render", async () => {
    vi.useFakeTimers();
    backend();
    render(<PeerSidebar refreshInterval={1000} />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2500);
    });
    const listCalls = mockInvoke.mock.calls.filter(([c]) => c === "peer_list_peers").length;
    // Initial load + two ticks; a re-render loop would make hundreds.
    expect(listCalls).toBeGreaterThanOrEqual(2);
    expect(listCalls).toBeLessThanOrEqual(4);
  });

  it("sends the chosen paths to the picked device", async () => {
    backend({ peer_request_transfer: { ...request, request_id: "out1" } });
    render(<PeerSidebar sendPaths={["/home/me/a.txt", "/home/me/photos"]} />);
    expect(screen.getByTestId("peer-send-banner").textContent).toContain("2 items");
    fireEvent.click(await screen.findByLabelText("Transfer to studio-mac"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("peer_request_transfer", { peerId: "p1", paths: ["/home/me/a.txt", "/home/me/photos"] }));
  });

  it("shows a send failure, and no send button without files", async () => {
    backend({ peer_request_transfer: new Error("Peer studio-mac is blocked") });
    const { unmount } = render(<PeerSidebar sendPaths={["/a"]} />);
    fireEvent.click(await screen.findByLabelText("Transfer to studio-mac"));
    expect(await screen.findByRole("alert")).toHaveTextContent("blocked");
    unmount();
    backend();
    render(<PeerSidebar />);
    await screen.findByText("studio-mac");
    expect(screen.queryByLabelText("Transfer to studio-mac")).toBeNull();
  });

  it("renders transfer progress in the backend's shape and cancels", async () => {
    backend({ peer_list_transfers: [transfer, { ...transfer, request_id: "r2", status: "completed", direction: "incoming", save_path: "/home/me/Downloads" }] });
    render(<PeerSidebar />);
    const row = await screen.findByTestId("transfer-r1");
    expect(row.textContent).toContain("studio-mac");
    expect(row.textContent).toContain("(1/2 files)");
    expect(row.textContent).toContain("/s");
    expect(screen.getByTestId("transfer-r2").textContent).toContain("Saved in /home/me/Downloads");
    fireEvent.click(screen.getByLabelText("Cancel transfer to studio-mac"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("peer_cancel_transfer", { requestId: "r1" }));
    // Finished ones can be cleared; only the unfinished one offers Cancel.
    expect(screen.queryByLabelText("Cancel transfer from studio-mac")).toBeNull();
    fireEvent.click(screen.getByText("Clear finished"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("peer_clear_finished", undefined));
  });

  it("answers requests into Downloads or the current folder", async () => {
    backend({ peer_list_pending_requests: [request] });
    render(<PeerSidebar receiveDir="/home/me/Projects" />);
    const row = await screen.findByTestId("request-r9");
    expect(row.textContent).toContain("2 files");
    expect(row.textContent).toContain("album/");
    fireEvent.click(screen.getByText("Save here"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("peer_respond_transfer", { requestId: "r9", response: "accepted", savePath: "/home/me/Projects" }));
    fireEvent.click(screen.getByText("Accept"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("peer_respond_transfer", { requestId: "r9", response: "accepted", savePath: undefined }));
  });

  it("shows this device's fingerprint and discovery errors", async () => {
    backend({ peer_list_peers: [], peer_start_discovery: new Error("Network discovery is unavailable: no interfaces") });
    render(<PeerSidebar />);
    expect((await screen.findByTestId("peer-local-info")).textContent).toContain("0123 4567 89AB CDEF · receiving on port 43210");
    fireEvent.click(screen.getByText("Start Scanning"));
    expect(await screen.findByRole("alert")).toHaveTextContent("Network discovery is unavailable");
  });
});

describe("Send to another computer entry points", () => {
  it("appears in the file context menu only when wired", () => {
    const h = () => {};
    const base = {
      hasSelection: true,
      isDirectory: false,
      selectionCount: 2,
      onCopy: h, onCut: h, onPaste: h, onDelete: h, onRename: h, onDuplicate: h, onNewFolder: h,
      onNewFile: h, onAddToFavorites: h, onGetInfo: h, onOpen: h, onRefresh: h, onSelectAll: h, onInvertSelection: h,
    };
    const onSend = vi.fn();
    const item = getFileContextMenuItems({ ...base, onSendToComputer: onSend }).find((i) => i.id === "send-to-computer");
    expect(item?.label).toBe("Send to Another Computer…");
    item?.action?.();
    expect(onSend).toHaveBeenCalled();
    expect(getFileContextMenuItems(base).some((i) => i.id === "send-to-computer")).toBe(false);
  });

  it("appears in the command palette", () => {
    const onOpen = vi.fn();
    const commands = getDefaultCommands({ onOpenNearby: onOpen, onSendToComputer: vi.fn() });
    commands.find((c) => c.id === "open-nearby")!.action();
    expect(onOpen).toHaveBeenCalled();
    expect(commands.some((c) => c.id === "send-to-computer")).toBe(true);
    expect(getDefaultCommands({}).some((c) => c.id === "send-to-computer")).toBe(false);
  });
});
