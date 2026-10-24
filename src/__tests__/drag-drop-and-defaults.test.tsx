/**
 * Drag-and-drop confirmation (risk from the Safety Interlock), Settings ›
 * General (confirmation + default folders, persisted), the default folders
 * in the Connection Manager's download/upload pickers, and its "Open in
 * pane" actions.
 */
import { describe, it, expect, vi, beforeEach, beforeAll } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { DragDropConfirm } from "@/components/drag-drop-confirm";
import { dropIntent } from "@/lib/drag-drop";
import { GeneralSection } from "@/components/settings-panel";
import { ConnectionPanel } from "@/components/connection-panel";
import { useUIStore } from "@/stores/ui-store";

const CONN = { id: "6f1c2b0a-1111-4222-8333-944455556666", name: "Test SFTP", protocol: "sftp", host: "h", port: 22, username: "u", credential_ref: null, remote_path: "/srv", created_at: "", last_used: null, group_id: null, bandwidth_limit_bps: 0, verify_checksums: false };
const calls = (cmd: string) => mockInvoke.mock.calls.filter((c) => c[0] === cmd).map((c) => c[1]);
let risk = false;

beforeAll(() => {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

beforeEach(() => {
  risk = false;
  useUIStore.setState({ confirmDragDrop: false, defaultDownloadDir: null, defaultUploadDir: null });
  mockInvoke.mockReset().mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "safety_assess_intent":
        return risk
          ? { level: "high", reasons: ["Moves 40× more files than usual"], baseline_sample_size: 5, baseline_median_files: 2, baseline_median_bytes: 0, intent_hash: "h1", requires_confirmation: true }
          : { level: "low", reasons: [], baseline_sample_size: 0, baseline_median_files: 0, baseline_median_bytes: 0, intent_hash: "h0", requires_confirmation: false };
      case "list_connections":
        return [CONN];
      case "list_connection_groups":
      case "connector_list_protocols":
      case "list_connection_status":
        return [];
      case "get_connection":
        return CONN;
      case "connector_list_remote":
        return [{ name: "report.txt", path: "/srv/report.txt", is_dir: false, size: 5, modified: null }];
      case "connector_is_connected":
        return true;
      case "pick_files":
        return [];
      case "pick_folder":
        return null;
      default:
        return undefined;
    }
  });
});

const request = { sources: ["/a", "/b"], destDir: "/dst", mode: "move" as const, destLabel: "dst", totalBytes: 2048 };

describe("drag-and-drop confirmation", () => {
  it("summarises the drop and runs it only on confirm", async () => {
    const onConfirm = vi.fn();
    const onCancel = vi.fn();
    render(<DragDropConfirm request={request} onConfirm={onConfirm} onCancel={onCancel} />);
    expect(screen.getByTestId("drag-drop-summary")).toHaveTextContent("Move 2 items to dst? (2.0 KB)");
    await waitFor(() => expect(calls("safety_assess_intent")).toHaveLength(1));
    expect(calls("safety_assess_intent")[0]).toEqual({ intent: dropIntent(request) });
    expect(screen.queryByTestId("drag-drop-risk")).not.toBeInTheDocument();
    fireEvent.click(screen.getByTestId("drag-drop-ok"));
    expect(onConfirm).toHaveBeenCalledWith(expect.objectContaining({ requires_confirmation: false }));
    fireEvent.keyDown(window, { key: "Escape" });
    expect(onCancel).toHaveBeenCalled();
  });

  it("cannot be confirmed before the Safety Interlock has assessed the drop", async () => {
    // Regression: the autofocused button confirmed with no assessment (a
    // quick Enter), and the drop then skipped the interlock altogether.
    let answer: (a: unknown) => void = () => undefined;
    mockInvoke.mockImplementation((cmd: string) => (cmd === "safety_assess_intent" ? new Promise((r) => (answer = r)) : Promise.resolve(undefined)));
    const onConfirm = vi.fn();
    render(<DragDropConfirm request={request} onConfirm={onConfirm} onCancel={vi.fn()} />);
    const ok = screen.getByTestId("drag-drop-ok");
    expect(ok).toBeDisabled();
    fireEvent.click(ok);
    expect(onConfirm).not.toHaveBeenCalled();
    answer({ level: "high", reasons: ["Moves 40× more files than usual"], baseline_sample_size: 5, baseline_median_files: 2, baseline_median_bytes: 0, intent_hash: "h1", requires_confirmation: true });
    await waitFor(() => expect(ok).toBeEnabled());
    expect(ok).toHaveFocus();
    fireEvent.click(ok);
    expect(onConfirm).toHaveBeenCalledWith(expect.objectContaining({ requires_confirmation: true }));
  });

  it("shows the Safety Interlock's risk when it would ask", async () => {
    risk = true;
    render(<DragDropConfirm request={request} onConfirm={vi.fn()} onCancel={vi.fn()} />);
    const r = await screen.findByTestId("drag-drop-risk");
    expect(r).toHaveTextContent("High risk");
    expect(r).toHaveTextContent("Moves 40× more files than usual");
    expect(dropIntent({ ...request, mode: "copy" }).kind).toBe("copy");
  });
});

describe("Settings › General", () => {
  it("toggles the drag-and-drop confirmation and keeps default folders", () => {
    render(<GeneralSection />);
    const box = screen.getByTestId("setting-confirm-drag-drop");
    expect(box).not.toBeChecked();
    fireEvent.click(box);
    expect(useUIStore.getState().confirmDragDrop).toBe(true);
    fireEvent.change(screen.getByTestId("setting-download-dir"), { target: { value: "/home/me/Downloads" } });
    fireEvent.change(screen.getByTestId("setting-upload-dir"), { target: { value: "  " } });
    expect(useUIStore.getState().defaultDownloadDir).toBe("/home/me/Downloads");
    expect(useUIStore.getState().defaultUploadDir).toBeNull();
    // Persisted with the rest of the UI preferences.
    const persisted = useUIStore.persist.getOptions().partialize!(useUIStore.getState()) as Record<string, unknown>;
    expect(persisted).toMatchObject({ confirmDragDrop: true, defaultDownloadDir: "/home/me/Downloads", defaultUploadDir: null });
  });
});

describe("Connection Manager with default folders and Open in pane", () => {
  async function browse() {
    fireEvent.click(await screen.findByText("Test SFTP"));
    await screen.findByText("report.txt");
  }

  it("opens the upload and download pickers in the default folders", async () => {
    useUIStore.setState({ defaultDownloadDir: "/home/me/Downloads", defaultUploadDir: "/home/me/Outbox" });
    render(<ConnectionPanel />);
    await browse();
    fireEvent.click(screen.getByTestId("remote-upload"));
    await waitFor(() => expect(calls("pick_files")).toEqual([{ title: "Select files to upload", defaultPath: "/home/me/Outbox" }]));
    fireEvent.click(screen.getByTestId("remote-download-report.txt"));
    await waitFor(() => expect(calls("pick_folder")).toEqual([{ title: "Choose where to save report.txt", defaultPath: "/home/me/Downloads" }]));
  });

  it("opens a connection, or the folder being browsed, in the active pane", async () => {
    const onOpenInPane = vi.fn();
    render(<ConnectionPanel onOpenInPane={onOpenInPane} />);
    fireEvent.click(await screen.findByLabelText("Open Test SFTP in pane"));
    expect(onOpenInPane).toHaveBeenCalledWith(`conn://${CONN.id}/srv`, "Test SFTP");
    await browse();
    fireEvent.click(screen.getByTestId("remote-open-in-pane"));
    expect(onOpenInPane).toHaveBeenLastCalledWith(`conn://${CONN.id}/srv`, "Test SFTP");
  });
});
