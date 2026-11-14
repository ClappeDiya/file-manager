/**
 * The mount dialog and the Mounts list are reachable from the app shell:
 * the command palette ("Mount Drive…", "Show Mounts"), the toolbar item, and
 * the Connection Manager's "Mount as drive" row action. (The dialog used to be
 * imported only by tests, so no user could ever open it.)
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...a: unknown[]) => mockInvoke(...a),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async () => () => {},
  emit: async () => {},
}));

import { FileManager } from "@/components/file-manager";
import { ConnectionPanel } from "@/components/connection-panel";
import { MountsHost } from "@/components/mounts-host";
import { useFileManagerStore } from "@/stores/file-manager-store";
import { useMountsUi } from "@/stores/mounts-ui-store";
import { useUIStore } from "@/stores/ui-store";

const conn = (id: string, name: string, protocol: string) => ({
  id,
  name,
  protocol,
  host: "example.com",
  port: null,
  username: null,
  remote_path: "/",
  created_at: "2026-01-01T00:00:00Z",
  last_used: null,
  group_id: null,
});

const saved = {
  id: "m-saved",
  connection_id: "c-bucket",
  connection_name: "Photos bucket",
  protocol: "s3",
  mount_point: "/home/u/UFOP-Mounts/Photos",
  status: "unmounted",
  auto_mount: false,
  created_at: "2026-01-01T00:00:00Z",
  error_message: null,
  options: {
    cache_mode: "writes",
    max_cache_mb: null,
    max_cache_age_hours: null,
    read_only: false,
  },
  policy_notes: [],
};

beforeEach(() => {
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  mockInvoke.mockReset().mockImplementation(async (cmd: string) => {
    if (cmd === "list_connection_groups") return [];
    if (cmd === "list_connections")
      return [
        conn("c-bucket", "Photos bucket", "s3"),
        conn("c-nas", "Office NAS", "sftp"),
        conn("c-ftp", "Old FTP", "ftp"),
      ];
    if (cmd === "list_mounts") return [saved];
    if (cmd === "check_mount_capability")
      return { native_mount: { command: "rclone mount" } };
    if (cmd === "mount_cache_usage") return 0;
    // Everything else the shell asks for at start-up: nothing yet.
    return ["ledger_since_last_seen", "ledger_get_pulse"].includes(cmd)
      ? null
      : [];
  });
  useMountsUi.setState({
    panelOpen: false,
    dialogOpen: false,
    dialogConnectionId: null,
    editing: null,
    refreshTick: 0,
  });
  useFileManagerStore.setState({ commandPaletteOpen: false });
  useUIStore.setState({
    toolbarItems: [
      ...useUIStore.getState().toolbarItems.filter((i) => i !== "mounts"),
    ],
  });
});

describe("mounts from the app shell", () => {
  it("opens the mount dialog from the command palette", async () => {
    render(<FileManager />);
    expect(
      screen.queryByRole("dialog", { name: /Mount Remote Drive/ }),
    ).not.toBeInTheDocument();
    act(() => useFileManagerStore.getState().setCommandPaletteOpen(true));
    fireEvent.change(await screen.findByTestId("command-palette-input"), {
      target: { value: "mount" },
    });
    fireEvent.click(await screen.findByTestId("command-mount-drive"));
    const dialog = await screen.findByRole("dialog", {
      name: /Mount Remote Drive/,
    });
    // The saved connections load into the form.
    await waitFor(() =>
      expect(
        (within(dialog).getByLabelText("Connection") as HTMLSelectElement)
          .options.length,
      ).toBe(4),
    );
  });

  it("opens the Mounts list from the palette and from the toolbar item", async () => {
    render(<FileManager />);
    act(() => useFileManagerStore.getState().setCommandPaletteOpen(true));
    fireEvent.change(await screen.findByTestId("command-palette-input"), {
      target: { value: "mount" },
    });
    fireEvent.click(await screen.findByTestId("command-open-mounts"));
    const list = await screen.findByRole("dialog", { name: "Mounts" });
    expect(await within(list).findByText("Photos bucket")).toBeInTheDocument();
    act(() => useMountsUi.getState().closePanel());

    expect(screen.queryByTestId("toggle-mounts")).not.toBeInTheDocument();
    act(() =>
      useUIStore
        .getState()
        .setToolbarItems([...useUIStore.getState().toolbarItems, "mounts"]),
    );
    fireEvent.click(await screen.findByTestId("toggle-mounts"));
    expect(
      await screen.findByRole("dialog", { name: "Mounts" }),
    ).toBeInTheDocument();
  });
});

describe("Mount as drive in the Connection Manager", () => {
  it("opens the dialog with that connection selected, only for connections that can be mounted", async () => {
    render(
      <>
        <ConnectionPanel />
        <MountsHost />
      </>,
    );
    fireEvent.click(
      await screen.findByLabelText("Mount Photos bucket as drive"),
    );
    const dialog = await screen.findByRole("dialog", {
      name: /Mount Remote Drive/,
    });
    await waitFor(() =>
      expect(
        (within(dialog).getByLabelText("Connection") as HTMLSelectElement)
          .value,
      ).toBe("c-bucket"),
    );
    expect(
      screen.getByLabelText("Mount Office NAS as drive"),
    ).toBeInTheDocument();
    expect(
      screen.queryByLabelText("Mount Old FTP as drive"),
    ).not.toBeInTheDocument();
  });
});
