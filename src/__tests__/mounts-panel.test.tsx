/** Mounts list: state per mount and the actions that apply to it. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
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

import { MountsPanel } from "@/components/mounts-panel";
import { MountsHost } from "@/components/mounts-host";
import { useMountsUi } from "@/stores/mounts-ui-store";

const base = {
  auto_mount: false,
  created_at: "2026-01-01T00:00:00Z",
  error_message: null,
  policy_notes: [],
};
const opts = {
  cache_mode: "writes",
  max_cache_mb: null,
  max_cache_age_hours: null,
  read_only: false,
};
const mk = (
  id: string,
  name: string,
  status: string,
  extra: Record<string, unknown> = {},
) => ({
  ...base,
  id,
  connection_id: `c-${id}`,
  connection_name: name,
  protocol: "sftp",
  mount_point: `/mnt/${name}`,
  status,
  ...extra,
});

let mounts: ReturnType<typeof mk>[];

beforeEach(() => {
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  useMountsUi.setState({
    panelOpen: false,
    dialogOpen: false,
    dialogConnectionId: null,
    editing: null,
    refreshTick: 0,
  });
  mounts = [
    mk("m1", "Work", "mounted", {
      auto_mount: true,
      options: { ...opts, subfolder: "docs" },
    }),
    mk("m2", "Photos", "unmounted", { protocol: "s3", options: opts }),
    mk("m3", "Broken", "error", {
      error_message: "rclone mount exited (exit status: 1)",
    }),
  ];
  mockInvoke.mockReset().mockImplementation(async (cmd: string) => {
    if (cmd === "list_mounts") return mounts;
    if (cmd === "mount_cache_usage") return 0;
    if (cmd === "list_connections") return [];
    if (cmd === "mount_saved_config")
      return {
        mount_id: "m2",
        mount_point: "/mnt/Photos",
        status: "mounted",
        message: "Successfully mounted 'Photos'",
      };
    if (cmd === "unmount_all_mounts") return { unmounted: 1, failed: [] };
    return null;
  });
});

const row = (id: string) => within(screen.getByTestId(`mount-row-${id}`));
const calls = (cmd: string) =>
  mockInvoke.mock.calls.filter((c) => c[0] === cmd).map((c) => c[1]);

describe("Mounts list", () => {
  it("shows every mount with its state and only the actions that apply", async () => {
    render(<MountsPanel />);
    await screen.findByText("Work");
    expect(row("m1").getByText("Mounted")).toBeInTheDocument();
    expect(row("m1").getByText(/Folder: docs/)).toBeInTheDocument();
    expect(row("m1").getByText(/Auto-mount/)).toBeInTheDocument();
    expect(row("m2").getByText("Unmounted")).toBeInTheDocument();
    expect(row("m3").getByText("Error")).toBeInTheDocument();
    expect(
      row("m3").getByText("rclone mount exited (exit status: 1)"),
    ).toBeInTheDocument();

    // Mounted: open and unmount; not mount; edit waits for the unmount.
    expect(row("m1").getByLabelText("Open Work")).toBeEnabled();
    expect(row("m1").getByLabelText("Unmount Work")).toBeEnabled();
    expect(row("m1").queryByLabelText("Mount Work")).not.toBeInTheDocument();
    expect(row("m1").getByLabelText("Edit Work")).toBeDisabled();
    // Not mounted (or failed): mount and edit; nothing to open or unmount.
    for (const [id, name] of [
      ["m2", "Photos"],
      ["m3", "Broken"],
    ]) {
      expect(row(id).getByLabelText(`Mount ${name}`)).toBeEnabled();
      expect(row(id).getByLabelText(`Edit ${name}`)).toBeEnabled();
      expect(row(id).queryByLabelText(`Open ${name}`)).not.toBeInTheDocument();
      expect(
        row(id).queryByLabelText(`Unmount ${name}`),
      ).not.toBeInTheDocument();
    }
  });

  it("says so when there are no mounts", async () => {
    mounts = [];
    render(<MountsPanel />);
    expect(await screen.findByText(/No mounts yet/)).toBeInTheDocument();
    expect(screen.getByText("Unmount all")).toBeDisabled();
  });

  it("mounts a saved mount and reloads the list", async () => {
    render(<MountsPanel />);
    await screen.findByText("Photos");
    fireEvent.click(row("m2").getByLabelText("Mount Photos"));
    expect(await screen.findByRole("status")).toHaveTextContent(
      "Successfully mounted 'Photos'",
    );
    expect(calls("mount_saved_config")).toEqual([{ mountId: "m2" }]);
    await waitFor(() => expect(calls("list_mounts").length).toBeGreaterThan(1));
  });

  it("shows why a mount failed on its own row", async () => {
    const original = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation(async (cmd: string, a: unknown) => {
      if (cmd === "mount_saved_config")
        throw new Error("rclone is not installed");
      return original(cmd, a);
    });
    render(<MountsPanel />);
    await screen.findByText("Broken");
    fireEvent.click(row("m3").getByLabelText("Mount Broken"));
    expect(await row("m3").findByRole("alert")).toHaveTextContent(
      "rclone is not installed",
    );
    expect(row("m2").queryByRole("alert")).not.toBeInTheDocument();
  });

  // Tauri rejects with the backend's AppError payload, never an Error.
  const appError = (message: string) => ({
    error_type: "validation",
    message,
    advice: "Try again.",
  });

  it("shows the reason from the backend's error payload, not [object Object]", async () => {
    const original = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation(async (cmd: string, a: unknown) => {
      if (cmd === "mount_saved_config")
        throw appError("Unmount the drive before editing it");
      return original(cmd, a);
    });
    render(<MountsPanel />);
    await screen.findByText("Broken");
    fireEvent.click(row("m3").getByLabelText("Mount Broken"));
    const alert = await row("m3").findByRole("alert");
    expect(alert).toHaveTextContent("Unmount the drive before editing it");
    expect(alert).toHaveTextContent("Try again.");
    expect(alert).not.toHaveTextContent("[object Object]");
  });

  it("shows the backend's reason when unmount all fails", async () => {
    const original = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation(async (cmd: string, a: unknown) => {
      if (cmd === "unmount_all_mounts") throw appError("Nothing could be done");
      return original(cmd, a);
    });
    render(<MountsPanel />);
    await screen.findByText("Work");
    fireEvent.click(screen.getByText("Unmount all"));
    const note = await screen.findByRole("status");
    expect(note).toHaveTextContent("Nothing could be done");
    expect(note).not.toHaveTextContent("[object Object]");
  });

  it("reloads the list once per action", async () => {
    render(<MountsPanel />);
    await screen.findByText("Photos");
    await waitFor(() => expect(calls("list_mounts")).toHaveLength(1));
    fireEvent.click(row("m2").getByLabelText("Mount Photos"));
    await screen.findByRole("status");
    await waitFor(() => expect(calls("list_mounts")).toHaveLength(2));
    await new Promise((r) => setTimeout(r, 50));
    expect(calls("list_mounts")).toHaveLength(2);
  });

  it("unmounts and opens a mounted drive", async () => {
    render(<MountsPanel />);
    await screen.findByText("Work");
    fireEvent.click(row("m1").getByLabelText("Open Work"));
    await waitFor(() =>
      expect(calls("open_mount_point")).toEqual([{ mountId: "m1" }]),
    );
    fireEvent.click(row("m1").getByLabelText("Unmount Work"));
    await waitFor(() =>
      expect(calls("unmount_remote")).toEqual([{ mountId: "m1" }]),
    );
  });

  it("removes a mount only after confirming", async () => {
    render(<MountsPanel />);
    await screen.findByText("Photos");
    fireEvent.click(row("m2").getByLabelText("Remove Photos"));
    expect(calls("delete_mount_config")).toEqual([]);
    fireEvent.click(row("m2").getByText("No"));
    expect(calls("delete_mount_config")).toEqual([]);
    fireEvent.click(row("m2").getByLabelText("Remove Photos"));
    fireEvent.click(row("m2").getByLabelText("Confirm remove Photos"));
    await waitFor(() =>
      expect(calls("delete_mount_config")).toEqual([{ mountId: "m2" }]),
    );
  });

  it("unmounts everything and reports it", async () => {
    render(<MountsPanel />);
    await screen.findByText("Work");
    fireEvent.click(screen.getByText("Unmount all"));
    expect(await screen.findByRole("status")).toHaveTextContent(
      "Unmounted 1 drive.",
    );
    expect(calls("unmount_all_mounts")).toHaveLength(1);
    // Drives that refused are named.
    const original = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation(async (cmd: string, a: unknown) =>
      cmd === "unmount_all_mounts"
        ? { unmounted: 1, failed: ["Work: device is busy"] }
        : original(cmd, a),
    );
    fireEvent.click(screen.getByText("Unmount all"));
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent(
        "could not unmount: Work: device is busy",
      ),
    );
  });

  it("opens the mount dialog for a new mount and, prefilled, to edit a saved one", async () => {
    render(
      <>
        <MountsPanel />
        <MountsHost />
      </>,
    );
    await screen.findByText("Photos");
    fireEvent.click(screen.getByText("Mount drive…"));
    expect(
      await screen.findByRole("dialog", { name: "Mount Remote Drive" }),
    ).toBeInTheDocument();
    useMountsUi.getState().closeDialog();

    fireEvent.click(row("m3").getByLabelText("Edit Broken"));
    const dialog = await screen.findByRole("dialog", { name: "Edit Mount" });
    expect(useMountsUi.getState().editing?.id).toBe("m3");
    expect(dialog).toBeInTheDocument();
  });

  it("edits from the options the user saved, not the ones a profile forced", async () => {
    mounts = [
      mk("m3", "Broken", "error", {
        options: { ...opts, read_only: true, max_cache_mb: 32 },
      }),
    ];
    const original = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation(async (cmd: string, a: unknown) =>
      cmd === "saved_mount_options"
        ? { ...opts, read_only: false, max_cache_mb: 4096 }
        : original(cmd, a),
    );
    render(<MountsPanel />);
    await screen.findByText("Broken");
    fireEvent.click(row("m3").getByLabelText("Edit Broken"));
    await waitFor(() =>
      expect(useMountsUi.getState().editing?.options).toMatchObject({
        read_only: false,
        max_cache_mb: 4096,
      }),
    );
    expect(calls("saved_mount_options")).toEqual([{ mountId: "m3" }]);
  });

  it("reloads when a mount changes elsewhere", async () => {
    render(<MountsPanel />);
    await screen.findByText("Work");
    mounts = [mk("m9", "Elsewhere", "mounted")];
    useMountsUi.getState().bump();
    expect(await screen.findByText("Elsewhere")).toBeInTheDocument();
    expect(screen.queryByText("Work")).not.toBeInTheDocument();
  });
});
