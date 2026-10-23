/**
 * Cloud destination sign-in and advanced settings: defaults equal the Rust
 * `Default`s (same JSON as `cloud_config_defaults_match_frontend`), the S3
 * credential source / assume-role, Azure Entra sign-in, class-per-kind and
 * rclone editors write the snake_case shapes the Rust configs read, and the
 * point-in-time and request-limit controls call the backend correctly.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { CloudDestinationForm, newCloudConfig } from "@/components/backup/backup-cloud-destination";
import { PointInTimeOpen } from "@/components/backup/backup-point-in-time";
import { RequestLimitsEditor } from "@/components/backup/backup-request-limits";
import { azureAuthOf, s3CredentialsOf, secretHint } from "@/lib/backup-cloud-auth";
import { DEFAULT_REQUEST_LIMITS, newBackupJob, type BackupJob, type CloudConfig } from "@/stores/backup-store";
import { useUIStore } from "@/stores/ui-store";

const tls = { accept_invalid_certs: false, ca_pem: null, pinned_sha256: [] };
const byKind = { dblock: "", dindex: "", dlist: "", other: "" };

beforeEach(() => {
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  mockInvoke.mockImplementation(async (cmd: string) => {
    if (cmd === "backup_cloud_providers") return [];
    if (cmd === "backup_has_destination_secret") return false;
    return null;
  });
  act(() => useUIStore.setState({ appMode: "simple", backupAdvanced: null }));
});

describe("cloud sign-in defaults", () => {
  it("S3, Azure, GCS and rclone defaults equal the Rust Default impls", () => {
    expect(newCloudConfig("s3")).toEqual({
      provider: "s3",
      preset: "aws",
      endpoint: "",
      region: "us-east-1",
      bucket: "",
      prefix: "",
      access_key_id: "",
      path_style: false,
      storage_class: "",
      part_size_mb: 64,
      cold_storage: { restore_days: 3, tier: "Standard", wait_minutes: 0 },
      create_with_object_lock: false,
      tls,
      credentials: { type: "static" },
      assume_role: null,
      storage_class_by_kind: byKind,
      point_in_time: null,
    });
    expect(newCloudConfig("azure")).toEqual({ provider: "azure", account: "", container: "", prefix: "", endpoint: "", access_tier: "", block_size_mb: 4, rehydrate_priority: "Standard", tls, auth: { type: "shared_key" }, authority_host: "", access_tier_by_kind: byKind });
    expect(newCloudConfig("gcs")).toEqual({ provider: "gcs", bucket: "", prefix: "", project: "", storage_class: "", endpoint: "", checksums: true, tls, scope: "", storage_class_by_kind: byKind });
    expect(newCloudConfig("rclone")).toEqual({ provider: "rclone", remote: "", path: "", binary: "rclone", config_file: null, extra_args: [], timeout_secs: 0, startup_timeout_secs: 0, env: {}, isolate_env: false, env_passthrough: [] });
    expect(newBackupJob().request_limits).toEqual(DEFAULT_REQUEST_LIMITS);
  });

  it("switching methods keeps shared fields and drops the others", () => {
    expect(s3CredentialsOf("profile")).toEqual({ type: "profile", name: "", file: null });
    expect(s3CredentialsOf("auto", { type: "profile", name: "x", file: null })).toEqual({ type: "auto" });
    const sp = azureAuthOf("service_principal_secret", { type: "federated_token", tenant: "t", client_id: "c", token_file: "/f" });
    expect(sp).toEqual({ type: "service_principal_secret", tenant: "t", client_id: "c" });
    expect(azureAuthOf("managed_identity", sp)).toEqual({ type: "managed_identity", client_id: "c" });
    expect(secretHint({ ...newCloudConfig("s3"), credentials: { type: "auto" } } as CloudConfig)).toMatch(/Not needed/);
    expect(secretHint(newCloudConfig("s3"))).toBeNull();
    expect(secretHint({ ...newCloudConfig("azure"), auth: { type: "service_principal_certificate", tenant: "", client_id: "", cert_path: "" } } as CloudConfig)).toMatch(/PFX password/);
  });
});

describe("cloud sign-in editors", () => {
  it("S3: credential source is always visible; assume-role and classes need Advanced", () => {
    let cfg: CloudConfig = newCloudConfig("s3");
    const onChange = vi.fn((c: CloudConfig) => (cfg = c));
    const { rerender } = render(<CloudDestinationForm config={cfg} credentialId="c" onChange={onChange} />);
    expect(screen.queryByTestId("s3-assume-role")).toBeNull();
    fireEvent.change(screen.getByTestId("s3-credential-source"), { target: { value: "web_identity" } });
    expect(cfg.credentials).toEqual({ type: "web_identity", role_arn: "", token_file: "" });
    rerender(<CloudDestinationForm config={cfg} credentialId="c" onChange={onChange} />);
    fireEvent.change(screen.getByTestId("s3-wi-token"), { target: { value: "/var/run/token" } });
    expect(cfg.credentials).toMatchObject({ token_file: "/var/run/token" });
    act(() => useUIStore.setState({ appMode: "advanced" }));
    rerender(<CloudDestinationForm config={cfg} credentialId="c" onChange={onChange} />);
    fireEvent.change(screen.getByTestId("s3-role-arn"), { target: { value: "arn:aws:iam::1:role/B" } });
    expect(cfg.assume_role).toEqual({ role_arn: "arn:aws:iam::1:role/B", external_id: null, session_name: null, duration_secs: null, sts_endpoint: null, region: null });
    rerender(<CloudDestinationForm config={cfg} credentialId="c" onChange={onChange} />);
    fireEvent.change(screen.getByTestId("s3-role-external-id"), { target: { value: "ext" } });
    expect(cfg.assume_role).toMatchObject({ external_id: "ext" });
    fireEvent.change(screen.getByTestId("class-dindex"), { target: { value: "STANDARD" } });
    expect(cfg.storage_class_by_kind).toEqual({ ...byKind, dindex: "STANDARD" });
    // Archive tiers are not offered per kind.
    expect(screen.getByTestId("class-dblock").innerHTML).not.toContain("DEEP_ARCHIVE");
    // Clearing the role ARN turns assume-role off.
    rerender(<CloudDestinationForm config={cfg} credentialId="c" onChange={onChange} />);
    fireEvent.change(screen.getByTestId("s3-role-arn"), { target: { value: "" } });
    expect(cfg.assume_role).toBeNull();
  });

  it("Azure Entra sign-in, tiers and rclone process settings", () => {
    let cfg: CloudConfig = newCloudConfig("azure");
    const onChange = vi.fn((c: CloudConfig) => (cfg = c));
    act(() => useUIStore.setState({ appMode: "advanced" }));
    const { rerender, unmount } = render(<CloudDestinationForm config={cfg} credentialId="c" onChange={onChange} />);
    fireEvent.change(screen.getByTestId("azure-auth-type"), { target: { value: "service_principal_certificate" } });
    rerender(<CloudDestinationForm config={cfg} credentialId="c" onChange={onChange} />);
    fireEvent.change(screen.getByTestId("azure-tenant"), { target: { value: "tid" } });
    rerender(<CloudDestinationForm config={cfg} credentialId="c" onChange={onChange} />);
    fireEvent.change(screen.getByTestId("azure-cert-path"), { target: { value: "/etc/sp.pfx" } });
    expect(cfg.auth).toEqual({ type: "service_principal_certificate", tenant: "tid", client_id: "", cert_path: "/etc/sp.pfx" });
    expect(screen.getByText(/PFX password/)).toBeInTheDocument();
    fireEvent.change(screen.getByTestId("class-dlist"), { target: { value: "Hot" } });
    expect(cfg.access_tier_by_kind).toMatchObject({ dlist: "Hot" });
    expect(screen.getByTestId("class-dblock").innerHTML).not.toContain("Archive");
    unmount();

    let rc: CloudConfig = newCloudConfig("rclone");
    const onRc = vi.fn((c: CloudConfig) => (rc = c));
    const r2 = render(<CloudDestinationForm config={rc} credentialId="c" onChange={onRc} />);
    fireEvent.change(screen.getByTestId("rclone-startup-timeout"), { target: { value: "30" } });
    expect(rc.startup_timeout_secs).toBe(30);
    r2.rerender(<CloudDestinationForm config={rc} credentialId="c" onChange={onRc} />);
    fireEvent.blur(screen.getByTestId("rclone-env"), { target: { value: "RCLONE_TPSLIMIT=4\nHTTPS_PROXY=http://p:3128" } });
    expect(rc.env).toEqual({ RCLONE_TPSLIMIT: "4", HTTPS_PROXY: "http://p:3128" });
    r2.rerender(<CloudDestinationForm config={rc} credentialId="c" onChange={onRc} />);
    fireEvent.click(screen.getByLabelText(/Isolated environment/));
    expect(rc.isolate_env).toBe(true);
  });
});

describe("point in time and request limits", () => {
  const s3Job = (): BackupJob => ({ ...newBackupJob(), id: "j1", name: "Office", destination: { type: "cloud", config: newCloudConfig("s3"), credential_id: "c" } });

  it("opens a read-only job as of the chosen time", async () => {
    mockInvoke.mockImplementation(async (cmd: string) => (cmd === "backup_open_point_in_time" ? { ...s3Job(), id: "j2", name: "Office (as of …)", read_only: true, stats: { ...newBackupJob().stats, version_count: 3 } } : cmd === "backup_list_jobs" ? [] : null));
    render(<PointInTimeOpen job={s3Job()} />);
    expect(screen.getByTestId("pit-open")).toBeDisabled();
    fireEvent.change(screen.getByTestId("pit-time"), { target: { value: "2026-10-01T08:30" } });
    fireEvent.click(screen.getByTestId("pit-open"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_open_point_in_time", { jobId: "j1", at: new Date("2026-10-01T08:30").toISOString(), name: null }));
    expect(await screen.findByText(/with 3 version\(s\)/)).toBeInTheDocument();
  });

  it("is only offered for writable S3 jobs", () => {
    const { container, rerender } = render(<PointInTimeOpen job={{ ...s3Job(), read_only: true }} />);
    expect(container.innerHTML).toBe("");
    rerender(<PointInTimeOpen job={newBackupJob()} />);
    expect(container.innerHTML).toBe("");
  });

  it("edits request limits with whole numbers where needed", () => {
    const onChange = vi.fn();
    render(<RequestLimitsEditor value={DEFAULT_REQUEST_LIMITS} onChange={onChange} />);
    fireEvent.change(screen.getByTestId("limit-put_per_sec"), { target: { value: "2.5" } });
    expect(onChange).toHaveBeenLastCalledWith({ ...DEFAULT_REQUEST_LIMITS, put_per_sec: 2.5 });
    fireEvent.change(screen.getByTestId("limit-list_parallelism"), { target: { value: "200.7" } });
    expect(onChange).toHaveBeenLastCalledWith({ ...DEFAULT_REQUEST_LIMITS, list_parallelism: 64 });
  });
});
