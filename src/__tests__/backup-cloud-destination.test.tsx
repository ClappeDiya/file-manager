/**
 * Cloud destination form: provider defaults match the Rust configs, edits
 * keep snake_case keys (including nested ones), secrets go only to the
 * credential store.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { CloudDestinationForm, newCloudConfig, PROVIDER_FIELDS } from "@/components/backup/backup-cloud-destination";
import type { CloudConfig } from "@/stores/backup-store";

beforeEach(() => {
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  mockInvoke.mockImplementation(async (cmd: string) => {
    if (cmd === "backup_cloud_providers")
      return [
        { id: "s3", name: "S3", secret: "Secret access key", verified: "emulator", hidden: false },
        { id: "azure", name: "Azure", secret: "Account key", verified: "emulator", hidden: false },
        { id: "box", name: "Box", secret: "token", verified: "api-mock", hidden: true },
      ];
    if (cmd === "backup_has_destination_secret") return false;
    return null;
  });
});

describe("cloud destination", () => {
  it("defaults mirror the Rust provider configs", () => {
    expect(newCloudConfig("s3")).toMatchObject({ provider: "s3", region: "us-east-1", part_size_mb: 64, cold_storage: { restore_days: 3, tier: "Standard" } });
    expect(newCloudConfig("smb")).toMatchObject({ provider: "smb", min_protocol: "SMB3", binary: "smbclient" });
    expect(Object.keys(PROVIDER_FIELDS)).toEqual(expect.arrayContaining(["s3", "azure", "gcs", "b2", "swift", "dropbox", "google_drive", "box", "pcloud", "onedrive", "tahoe_lafs", "rclone", "smb"]));
  });

  it("edits fields, hides untested providers and stores the secret separately", async () => {
    let cfg: CloudConfig = newCloudConfig("s3");
    const onChange = vi.fn((c: CloudConfig) => (cfg = c));
    render(<CloudDestinationForm config={cfg} credentialId="cred-9" onChange={onChange} />);
    await screen.findByText("Status: tested with a local emulator");
    expect(screen.queryByText(/Box/)).toBeNull();
    fireEvent.click(screen.getByLabelText("Show untested providers"));
    expect(screen.getByText("Box (untested)")).toBeInTheDocument();
    fireEvent.change(screen.getByTestId("cloud-bucket"), { target: { value: "my-bucket" } });
    expect(cfg.bucket).toBe("my-bucket");
    fireEvent.change(screen.getByTestId("cloud-wait_minutes"), { target: { value: "30" } });
    expect((cfg.cold_storage as { wait_minutes: number }).wait_minutes).toBe(30);
    fireEvent.change(screen.getByTestId("cloud-secret"), { target: { value: "s3cr3t" } });
    fireEvent.click(screen.getByTestId("cloud-secret-save"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_set_destination_secret", { credentialId: "cred-9", secret: "s3cr3t" }));
    expect(JSON.stringify(cfg)).not.toContain("s3cr3t");
  });
});
