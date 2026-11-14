/**
 * The in-flow folder picker where paths are chosen: the mount dialog's
 * subfolder (locked to the chosen connection, stored relative to its
 * folder) and a backup's folder on a connection.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import MountDriveDialog from "@/components/mount-drive-dialog";
import { useConnectionsStore } from "@/stores/connections-store";

const NAS = "0b5d8c3e-aaaa-4bbb-8ccc-dddddddddddd";
const conn = { id: NAS, name: "Office NAS", protocol: "sftp", host: "h", port: null, username: null, remote_path: "/data" };
const dir = (path: string, name: string) => ({ name, path, is_dir: true, is_symlink: false, size: 0, modified: null, created: null, is_hidden: false, extension: null, permissions: null });
const calls = (cmd: string) => mockInvoke.mock.calls.filter((c) => c[0] === cmd).map((c) => c[1]);

beforeEach(() => {
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  useConnectionsStore.setState({ connections: [], statuses: {}, loaded: false });
  mockInvoke.mockReset().mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    if (cmd === "list_connections") return [conn];
    if (cmd === "list_connection_status") return [];
    if (cmd === "check_mount_capability") return { native_mount: { command: "rclone mount" } };
    if (cmd === "connector_list_remote") return args?.path === "/data" ? [dir("/data/projects", "projects")] : args?.path === "/data/projects" ? [dir("/data/projects/2024", "2024")] : [];
    return null;
  });
});

describe("folder picker in the mount dialog", () => {
  it("picks a subfolder of the chosen connection, stored relative to its folder", async () => {
    const view = render(<MountDriveDialog open onClose={() => {}} />);
    const select = (await waitFor(() => {
      const s = view.container.querySelector("select")!;
      expect(s.options.length).toBe(2);
      return s;
    })) as HTMLSelectElement;
    fireEvent.change(select, { target: { value: NAS } });
    fireEvent.click(await screen.findByTestId("mount-subfolder-pick"));
    fireEvent.click(await screen.findByLabelText("Expand projects"));
    fireEvent.click(await screen.findByText("2024"));
    fireEvent.click(screen.getByTestId("picker-choose"));
    await waitFor(() => expect(screen.getByLabelText("Subfolder (optional)")).toHaveValue("projects/2024"));
    // Browsed through the connector, starting at the connection's folder.
    expect(calls("connector_list_remote")[0]).toEqual({ protocol: "sftp", path: "/data", connectionId: NAS });
    // No local Browse… for a connection subfolder.
    expect(screen.queryByText("Browse…")).not.toBeInTheDocument();
  });
});
