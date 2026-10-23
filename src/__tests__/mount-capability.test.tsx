/** Mount dialog: capability badge per connection; unsupported ones can't be mounted. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import MountDriveDialog from "@/components/mount-drive-dialog";

const conn = (id: string, protocol: string) => ({ id, name: id, protocol, host: "h", port: null, username: null, remote_path: "/" });

beforeEach(() => {
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  mockInvoke.mockReset().mockImplementation(async (cmd: string, a: { protocol: string }) => {
    if (cmd === "list_connections") return [conn("bucket", "s3"), conn("drive", "dropbox"), conn("nas", "sftp")];
    if (cmd === "list_mounts") return [];
    if (cmd === "check_mount_capability")
      return a.protocol === "s3"
        ? { native_mount: { command: "rclone mount" } }
        : a.protocol === "sftp"
          ? { requires_install: { tool: "sshfs", install_hint: "sudo apt install sshfs" } }
          : { not_supported: { reason: "Add this account as an rclone remote" } };
    return null;
  });
});

describe("mount capability badge", () => {
  it("shows what each connection needs and blocks unsupported mounts", async () => {
    const { container } = render(<MountDriveDialog open onClose={() => {}} />);
    const select = await waitFor(() => {
      const s = container.querySelector("select")!;
      expect(s.options.length).toBe(4);
      return s;
    });
    const mountButton = () => [...container.querySelectorAll("button")].find((b) => b.getAttribute("type") === "submit") as HTMLButtonElement;

    fireEvent.change(select, { target: { value: "bucket" } });
    expect((await screen.findByTestId("mount-capability")).textContent).toBe("Can be mounted (uses rclone mount)");
    expect(mountButton().disabled).toBe(false);

    fireEvent.change(select, { target: { value: "nas" } });
    await waitFor(() => expect(screen.getByTestId("mount-capability").textContent).toContain("sudo apt install sshfs"));
    expect(mountButton().disabled).toBe(true);

    fireEvent.change(select, { target: { value: "drive" } });
    await waitFor(() => expect(screen.getByTestId("mount-capability").textContent).toContain("rclone remote"));
    expect(mountButton().disabled).toBe(true);
  });
});
