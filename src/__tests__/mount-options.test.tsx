/**
 * Cloud mount options: cache fields only for rclone/S3 drives, locked where
 * the organization's mount profile forces them, sent with mount_remote and
 * save_mount_config; offline pinning and cache clearing per mount.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import MountDriveDialog from "@/components/mount-drive-dialog";
import { MountOfflineActions } from "@/components/mount-options";

const conn = (id: string, protocol: string) => ({ id, name: id, protocol, host: "h", port: null, username: null, remote_path: "/" });

beforeEach(() => {
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  mockInvoke.mockReset();
});

describe("mount dialog cache options", () => {
  it("shows cache options for cloud drives, locks what the profile forces and sends the options", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "list_connections") return [conn("bucket", "s3"), conn("nas", "sftp")];
      if (cmd === "list_mounts") return [];
      if (cmd === "check_mount_capability") return { native_mount: { command: "rclone mount" } };
      if (cmd === "mount_policy") return { allowedProtocols: null, readOnly: true, cacheMode: "full", maxCacheMb: 2048, maxCacheAgeHours: null, mountRoot: "~/Drives" };
      if (cmd === "mount_remote") return { mount_id: "m1", mount_point: "/home/u/Drives/bucket", status: "mounted", message: "Mounted (read-only (organization policy))" };
      return null;
    });
    const { container } = render(<MountDriveDialog open onClose={() => {}} />);
    const select = await waitFor(() => {
      const s = container.querySelector("select")!;
      expect(s.options.length).toBe(3);
      return s;
    });
    expect(await screen.findByTestId("mount-policy")).toHaveTextContent("mount points inside ~/Drives · read-only · cache full · cache ≤ 2048 MB");

    fireEvent.change(select, { target: { value: "nas" } });
    await screen.findByTestId("mount-capability");
    expect(screen.queryByLabelText("Cache options")).not.toBeInTheDocument();

    fireEvent.change(select, { target: { value: "bucket" } });
    expect(await screen.findByLabelText("Cache options")).toBeInTheDocument();
    expect(screen.getByLabelText("Cache mode")).toBeDisabled();
    expect(screen.getByLabelText("Cache mode")).toHaveValue("full");
    expect(screen.getByLabelText("Read-only")).toBeChecked();
    expect(screen.getByLabelText("Read-only")).toBeDisabled();
    expect(screen.getByText(/stay readable without a connection/)).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Largest cache"), { target: { value: "500" } });
    fireEvent.change(screen.getByLabelText("Keep cached files"), { target: { value: "48" } });
    fireEvent.click(screen.getByText("Auto-mount on startup"));
    fireEvent.click(screen.getByText("Mount"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("save_mount_config", expect.anything()));
    const options = { cache_mode: "writes", max_cache_mb: 500, max_cache_age_hours: 48, read_only: false };
    expect(mockInvoke).toHaveBeenCalledWith("mount_remote", { connectionId: "bucket", mountPoint: "~/UFOP-Mounts/bucket", options });
    expect(mockInvoke).toHaveBeenCalledWith("save_mount_config", { connectionId: "bucket", mountPoint: "~/UFOP-Mounts/bucket", autoMount: true, options });
    expect(await screen.findByText(/organization policy/)).toBeInTheDocument();
  });
});

describe("offline actions", () => {
  it("pins a folder of a full-cache mount and reports it", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "mount_cache_usage") return 3 * 1024 * 1024;
      if (cmd === "mount_pin_offline") return { files: 2, bytes: 300004, failed: 1, truncated: false };
      return null;
    });
    render(<MountOfflineActions mountId="m1" mounted options={{ cache_mode: "full", max_cache_mb: null, max_cache_age_hours: null, read_only: true }} />);
    expect(await screen.findByText(/3.0 MB on this computer/)).toHaveTextContent("Cache: full · read-only");
    fireEvent.change(screen.getByLabelText("Folder to keep offline"), { target: { value: "projects/q3" } });
    fireEvent.click(screen.getByText("Make available offline"));
    expect(await screen.findByRole("status")).toHaveTextContent("2 files (293.0 KB) available offline, 1 could not be read.");
    expect(mockInvoke).toHaveBeenCalledWith("mount_pin_offline", { mountId: "m1", path: "projects/q3" });
  });

  it("offers no pinning without the full cache, and clears the cache of an unmounted drive", async () => {
    let usage = 1024;
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "mount_cache_usage") return usage;
      if (cmd === "mount_clear_cache") {
        const freed = usage;
        usage = 0;
        return freed;
      }
      return null;
    });
    const { rerender } = render(<MountOfflineActions mountId="m2" mounted options={{ cache_mode: "writes", max_cache_mb: null, max_cache_age_hours: null, read_only: false }} />);
    await screen.findByText(/1.0 KB on this computer/);
    expect(screen.queryByText("Make available offline")).not.toBeInTheDocument();
    expect(screen.queryByText("Clear cache")).not.toBeInTheDocument();
    rerender(<MountOfflineActions mountId="m2" mounted={false} options={{ cache_mode: "writes", max_cache_mb: null, max_cache_age_hours: null, read_only: false }} />);
    fireEvent.click(await screen.findByText("Clear cache"));
    expect(await screen.findByRole("status")).toHaveTextContent("Freed 1.0 KB.");
    expect(mockInvoke).toHaveBeenCalledWith("mount_clear_cache", { mountId: "m2" });
  });
});
