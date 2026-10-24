/**
 * Mount dialog: subfolder, "open after mounting", the Advanced section (per
 * OS, Advanced mode only), editing a saved mount, and what is sent.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...a: unknown[]) => mockInvoke(...a),
}));

import MountDriveDialog from "@/components/mount-drive-dialog";
import { MountAdvancedFields } from "@/components/mount-options";
import {
  cleanMountOptions,
  DEFAULT_MOUNT_OPTIONS,
  mountPayloadOptions,
  type MountInfo,
} from "@/lib/mount-options";
import { useUIStore } from "@/stores/ui-store";

const conn = (id: string, protocol: string) => ({
  id,
  name: id,
  protocol,
  host: "h",
  port: null,
  username: null,
  remote_path: "/data",
});
const calls = (cmd: string) =>
  mockInvoke.mock.calls.filter((c) => c[0] === cmd).map((c) => c[1]);

beforeEach(() => {
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  useUIStore.setState({ appMode: "simple" });
  mockInvoke.mockReset().mockImplementation(async (cmd: string) => {
    if (cmd === "list_connections")
      return [conn("bucket", "s3"), conn("nas", "sftp")];
    if (cmd === "check_mount_capability")
      return { native_mount: { command: "rclone mount" } };
    if (cmd === "mount_remote")
      return {
        mount_id: "m1",
        mount_point: "/p",
        status: "mounted",
        message: "Successfully mounted",
      };
    if (cmd === "free_drive_letters") return ["E", "F", "Z"];
    return null;
  });
});

async function openForm(
  id: string,
  props: Partial<React.ComponentProps<typeof MountDriveDialog>> = {},
) {
  const view = render(<MountDriveDialog open onClose={() => {}} {...props} />);
  const select = (await waitFor(() => {
    const s = view.container.querySelector("select")!;
    expect(s.options.length).toBe(3);
    return s;
  })) as HTMLSelectElement;
  if (!props.connectionId && !props.editing)
    fireEvent.change(select, { target: { value: id } });
  await screen.findByTestId("mount-capability");
  return view;
}

describe("clean options", () => {
  it("sends the four cache options and only the advanced ones that are set", () => {
    expect(cleanMountOptions(DEFAULT_MOUNT_OPTIONS)).toEqual(
      DEFAULT_MOUNT_OPTIONS,
    );
    expect(
      cleanMountOptions({
        ...DEFAULT_MOUNT_OPTIONS,
        subfolder: " ",
        volume_label: "",
        network_mode: false,
        extra_flags: [],
        uid: null,
        open_after_mount: false,
      }),
    ).toEqual(DEFAULT_MOUNT_OPTIONS);
    expect(
      cleanMountOptions({
        ...DEFAULT_MOUNT_OPTIONS,
        subfolder: " a/b ",
        uid: 0,
        extra_flags: ["--no-seek"],
        network_mode: true,
      }),
    ).toEqual({
      ...DEFAULT_MOUNT_OPTIONS,
      subfolder: "a/b",
      uid: 0,
      extra_flags: ["--no-seek"],
      network_mode: true,
    });
  });

  it("native mounts send options only for the subfolder and open-after-mount", () => {
    expect(mountPayloadOptions(false, DEFAULT_MOUNT_OPTIONS)).toBeNull();
    expect(
      mountPayloadOptions(false, {
        ...DEFAULT_MOUNT_OPTIONS,
        subfolder: "docs",
      }),
    ).toMatchObject({ subfolder: "docs" });
    expect(
      mountPayloadOptions(false, {
        ...DEFAULT_MOUNT_OPTIONS,
        open_after_mount: true,
      }),
    ).toMatchObject({ open_after_mount: true });
    expect(mountPayloadOptions(true, DEFAULT_MOUNT_OPTIONS)).toEqual(
      DEFAULT_MOUNT_OPTIONS,
    );
  });
});

describe("subfolder and open after mounting", () => {
  it("mounts a subfolder of an SFTP connection and asks to open it", async () => {
    await openForm("nas");
    fireEvent.change(screen.getByLabelText("Subfolder (optional)"), {
      target: { value: "projects/2024" },
    });
    fireEvent.click(screen.getByText("Open the drive after mounting"));
    fireEvent.click(screen.getByText("Mount"));
    await waitFor(() => expect(calls("mount_remote")).toHaveLength(1));
    expect(calls("mount_remote")[0]).toEqual({
      connectionId: "nas",
      mountPoint: "~/UFOP-Mounts/nas",
      options: {
        ...DEFAULT_MOUNT_OPTIONS,
        subfolder: "projects/2024",
        open_after_mount: true,
      },
    });
    expect(await screen.findByRole("status")).toHaveTextContent(
      "Successfully mounted",
    );
  });

  it("keeps sending no options for a plain native mount", async () => {
    await openForm("nas");
    fireEvent.click(screen.getByText("Mount"));
    await waitFor(() => expect(calls("mount_remote")).toHaveLength(1));
    expect(calls("mount_remote")[0]).toMatchObject({ options: null });
  });

  it("shows why a mount failed", async () => {
    const original = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation(async (cmd: string, a: unknown) => {
      if (cmd === "mount_remote")
        throw new Error("Mount point is already in use");
      return original(cmd, a);
    });
    await openForm("nas");
    fireEvent.click(screen.getByText("Mount"));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "already in use",
    );
  });
});

describe("Advanced section", () => {
  it("is hidden in Simple mode and shown for cloud drives in Advanced mode", async () => {
    await openForm("bucket");
    expect(screen.queryByTestId("mount-advanced")).not.toBeInTheDocument();
  });

  it("sends the advanced options of a cloud drive, and nothing else new", async () => {
    useUIStore.setState({ appMode: "advanced" });
    await openForm("bucket");
    expect(await screen.findByTestId("mount-advanced")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("File permissions"), {
      target: { value: "0640" },
    });
    fireEvent.change(screen.getByLabelText("Directory cache"), {
      target: { value: "5m" },
    });
    fireEvent.change(screen.getByLabelText("Cache folder"), {
      target: { value: "/data/cache" },
    });
    fireEvent.change(screen.getByLabelText("Extra rclone flags"), {
      target: { value: "--buffer-size=64M --no-modtime" },
    });
    fireEvent.click(screen.getByText("Mount"));
    await waitFor(() => expect(calls("mount_remote")).toHaveLength(1));
    expect(calls("mount_remote")[0]).toMatchObject({
      options: {
        ...DEFAULT_MOUNT_OPTIONS,
        file_perms: "0640",
        dir_cache_time: "5m",
        cache_dir: "/data/cache",
        extra_flags: ["--buffer-size=64M", "--no-modtime"],
      },
    });
  });

  it("does not offer advanced options for a native mount", async () => {
    useUIStore.setState({ appMode: "advanced" });
    await openForm("nas");
    expect(screen.queryByTestId("mount-advanced")).not.toBeInTheDocument();
  });
});

describe("Advanced fields per operating system", () => {
  const show = (os: "windows" | "mac" | "linux") =>
    render(
      <MountAdvancedFields
        value={DEFAULT_MOUNT_OPTIONS}
        onChange={() => {}}
        policy={null}
        os={os}
      />,
    );

  it("Windows: drive letter (free ones), label and network drive; no owner or allow-other", async () => {
    show("windows");
    const letters = (await screen.findByLabelText(
      "Drive letter",
    )) as HTMLSelectElement;
    await waitFor(() =>
      expect([...letters.options].map((o) => o.value)).toEqual([
        "",
        "E",
        "F",
        "Z",
      ]),
    );
    expect(screen.getByLabelText("Volume label")).toBeInTheDocument();
    expect(screen.getByLabelText("Network drive")).toBeInTheDocument();
    expect(
      screen.queryByLabelText("Allow other users"),
    ).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Owner")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Group")).not.toBeInTheDocument();
  });

  it("macOS: label, allow-other, owner and group; no drive letter or network drive", () => {
    show("mac");
    expect(screen.getByLabelText("Volume label")).toBeInTheDocument();
    expect(screen.getByLabelText("Allow other users")).toBeInTheDocument();
    expect(screen.getByLabelText("Owner")).toBeInTheDocument();
    expect(screen.queryByLabelText("Drive letter")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("Network drive")).not.toBeInTheDocument();
    expect(
      mockInvoke.mock.calls.some((c) => c[0] === "free_drive_letters"),
    ).toBe(false);
  });

  it("Linux: no volume label (rclone ignores it there)", () => {
    show("linux");
    expect(screen.queryByLabelText("Volume label")).not.toBeInTheDocument();
    expect(screen.getByLabelText("Allow other users")).toBeInTheDocument();
  });

  it("an organization mount root rules out a drive letter", async () => {
    render(
      <MountAdvancedFields
        value={DEFAULT_MOUNT_OPTIONS}
        onChange={() => {}}
        os="windows"
        policy={{
          allowedProtocols: null,
          readOnly: null,
          cacheMode: null,
          maxCacheMb: null,
          maxCacheAgeHours: null,
          mountRoot: "~/Drives",
        }}
      />,
    );
    expect(await screen.findByLabelText("Drive letter")).toBeDisabled();
    expect(
      screen.getByText(/requires mount points inside ~\/Drives/),
    ).toBeInTheDocument();
  });

  it("a chosen drive letter is sent as the mount point", async () => {
    useUIStore.setState({ appMode: "advanced" });
    const win = vi
      .spyOn(window.navigator, "platform", "get")
      .mockReturnValue("Win32");
    try {
      await openForm("bucket");
      fireEvent.change(await screen.findByLabelText("Drive letter"), {
        target: { value: "E" },
      });
      expect(screen.queryByLabelText("Mount Point")).not.toBeInTheDocument();
      fireEvent.click(screen.getByText("Mount"));
      await waitFor(() => expect(calls("mount_remote")).toHaveLength(1));
      expect(calls("mount_remote")[0]).toMatchObject({
        mountPoint: "E:",
        options: { drive_letter: "E" },
      });
    } finally {
      win.mockRestore();
    }
  });
});

describe("editing a saved mount", () => {
  const saved: MountInfo = {
    id: "m-saved",
    connection_id: "bucket",
    connection_name: "bucket",
    protocol: "s3",
    mount_point: "/home/u/Drives/bucket",
    status: "unmounted",
    auto_mount: true,
    created_at: "2026-01-01T00:00:00Z",
    error_message: null,
    options: {
      ...DEFAULT_MOUNT_OPTIONS,
      cache_mode: "full",
      subfolder: "photos",
      open_after_mount: true,
    },
    policy_notes: [],
  };

  it("fills in the saved settings and saves them with update_mount_config", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "list_connections")
        return [conn("bucket", "s3"), conn("nas", "sftp")];
      if (cmd === "check_mount_capability")
        return { native_mount: { command: "rclone mount" } };
      return null;
    });
    await openForm("bucket", { editing: saved });
    expect(
      screen.getByRole("heading", { name: "Edit Mount" }),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("Mount Point")).toHaveValue(
      "/home/u/Drives/bucket",
    );
    expect(screen.getByLabelText("Subfolder (optional)")).toHaveValue("photos");
    expect(screen.getByLabelText("Cache mode")).toHaveValue("full");
    expect(
      screen.getByLabelText("Open the drive after mounting"),
    ).toBeChecked();
    expect(screen.getByLabelText("Auto-mount on startup")).toBeChecked();

    fireEvent.change(screen.getByLabelText("Subfolder (optional)"), {
      target: { value: "photos/2024" },
    });
    fireEvent.click(screen.getByText("Save changes"));
    expect(await screen.findByRole("status")).toHaveTextContent("Saved");
    expect(calls("update_mount_config")).toEqual([
      {
        mountId: "m-saved",
        connectionId: "bucket",
        mountPoint: "/home/u/Drives/bucket",
        autoMount: true,
        options: {
          ...DEFAULT_MOUNT_OPTIONS,
          cache_mode: "full",
          subfolder: "photos/2024",
          open_after_mount: true,
        },
      },
    ]);
    expect(calls("mount_remote")).toEqual([]);
  });

  it("shows the backend's reason when saving fails, not [object Object]", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "list_connections")
        return [conn("bucket", "s3"), conn("nas", "sftp")];
      if (cmd === "check_mount_capability")
        return { native_mount: { command: "rclone mount" } };
      if (cmd === "update_mount_config")
        throw {
          error_type: "validation",
          message: "Unmount the drive before editing it",
          advice: "Unmount it from the Mounts list first.",
        };
      return null;
    });
    await openForm("bucket", { editing: saved });
    fireEvent.click(screen.getByText("Save changes"));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("Unmount the drive before editing it");
    expect(alert).not.toHaveTextContent("[object Object]");
  });

  it("keeps the saved mount point when the connection is the same (no suggested name)", async () => {
    await openForm("bucket", { editing: saved });
    expect(screen.getByLabelText("Mount Point")).toHaveValue(
      "/home/u/Drives/bucket",
    );
  });

  it("in Standard mode a saved drive letter shows and can be switched back to a folder", async () => {
    const lettered: MountInfo = {
      ...saved,
      mount_point: "E:",
      options: {
        ...DEFAULT_MOUNT_OPTIONS,
        ...saved.options,
        drive_letter: "E",
      },
    };
    await openForm("bucket", { editing: lettered });
    // No folder field and no Advanced section: the letter is shown, with a way out.
    expect(screen.queryByLabelText("Mount Point")).not.toBeInTheDocument();
    expect(screen.getByTestId("mount-drive-letter-note")).toHaveTextContent(
      "Drive E:",
    );
    fireEvent.click(screen.getByText("Use a folder instead"));
    expect(screen.getByLabelText("Mount Point")).toHaveValue("");
    fireEvent.change(screen.getByLabelText("Mount Point"), {
      target: { value: "/home/u/Drives/bucket" },
    });
    fireEvent.click(screen.getByText("Save changes"));
    await waitFor(() => expect(calls("update_mount_config")).toHaveLength(1));
    const sent = calls("update_mount_config")[0] as {
      mountPoint: string;
      options: Record<string, unknown>;
    };
    expect(sent.mountPoint).toBe("/home/u/Drives/bucket");
    expect(sent.options).not.toHaveProperty("drive_letter");
  });
});
