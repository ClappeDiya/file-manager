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
    expect(Object.keys(PROVIDER_FIELDS)).toEqual(expect.arrayContaining(["s3", "azure", "gcs", "b2", "swift", "dropbox", "google_drive", "box", "pcloud", "onedrive", "tahoe_lafs", "rclone", "smb", "filen", "drime", "movistar_cloud"]));
  });

  it("Filen, Drime and Movistar defaults equal the Rust Default impls", () => {
    // Same JSON as `cloud_config_defaults_match_frontend` in stores/tests.rs.
    const tls = { accept_invalid_certs: false, ca_pem: null, pinned_sha256: [] };
    expect(newCloudConfig("filen")).toEqual({ provider: "filen", email: "", folder: "", use_trash: false, gateway_url: "", ingest_url: "", egest_url: "", tls });
    expect(newCloudConfig("drime")).toEqual({ provider: "drime", folder: "", workspace_id: 0, page_size: 50, soft_delete: false, api_url: "", tls });
    expect(newCloudConfig("movistar_cloud")).toEqual({
      provider: "movistar_cloud",
      email: "",
      device_id: "",
      folder: "",
      list_limit: 2000,
      wait_for_validation: true,
      validation_timeout_secs: 600,
      validation_poll_secs: 2,
      base_url: "",
      upload_url: "",
      tls,
    });
    // Every provider with a TLS block gets the pinning fields.
    for (const p of ["filen", "drime", "movistar_cloud"]) expect(PROVIDER_FIELDS[p].fields.some((f) => f.key === "pinned_sha256")).toBe(true);
  });

  it("edits the Filen fields and keeps the password out of the config", async () => {
    let cfg: CloudConfig = newCloudConfig("filen");
    const onChange = vi.fn((c: CloudConfig) => (cfg = c));
    const base = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation(async (cmd: string, a: unknown) =>
      cmd === "backup_cloud_providers" ? [{ id: "filen", name: "Filen (end-to-end encrypted)", secret: "Account password", verified: "api-mock", hidden: false }] : base(cmd, a),
    );
    render(<CloudDestinationForm config={cfg} credentialId="cred-f" onChange={onChange} />);
    await screen.findByText(/API-level tests only/);
    expect(screen.getByText("Filen (end-to-end encrypted) (untested)")).toBeInTheDocument();
    fireEvent.change(screen.getByTestId("cloud-email"), { target: { value: "me@example.com" } });
    expect(cfg.email).toBe("me@example.com");
    fireEvent.change(screen.getByTestId("cloud-folder"), { target: { value: "Backups/pc" } });
    expect(cfg).toMatchObject({ provider: "filen", folder: "Backups/pc", use_trash: false, gateway_url: "" });
    fireEvent.click(screen.getByLabelText("Move deleted volumes to the Filen trash"));
    expect(cfg.use_trash).toBe(true);
    fireEvent.change(screen.getByTestId("cloud-secret"), { target: { value: "hunter2" } });
    fireEvent.click(screen.getByTestId("cloud-secret-save"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_set_destination_secret", { credentialId: "cred-f", secret: "hunter2" }));
    expect(JSON.stringify(cfg)).not.toContain("hunter2");
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

describe("certificate pinning", () => {
  it("fetches a server fingerprint and pins it into tls.pinned_sha256", async () => {
    const fp = "ab".repeat(32);
    const base = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation(async (cmd: string, a: unknown) => (cmd === "tls_fetch_fingerprint" ? fp : base(cmd, a)));
    const onChange = vi.fn();
    render(<CloudDestinationForm config={newCloudConfig("s3")} credentialId="c1" onChange={onChange} />);
    fireEvent.change(screen.getByTestId("fingerprint-target"), { target: { value: "https://minio.local:9000/bucket" } });
    fireEvent.click(screen.getByTestId("fingerprint-get"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("tls_fetch_fingerprint", { host: "minio.local", port: 9000 }));
    fireEvent.click(await screen.findByTestId("fingerprint-pin"));
    expect(onChange).toHaveBeenLastCalledWith(expect.objectContaining({ provider: "s3", tls: { accept_invalid_certs: false, ca_pem: null, pinned_sha256: [fp] } }));
    expect(PROVIDER_FIELDS.azure.fields.some((f) => f.key === "pinned_sha256")).toBe(true);
  });
});
