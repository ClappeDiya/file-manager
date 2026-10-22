/**
 * Notifications, log outputs and support tools: the shapes sent to Rust
 * match the serde types (flattened `type` tag, snake_case TLS modes,
 * secrets only through the secure store).
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { LogOutputsEditor, NotificationsEditor, ReportExtras, newNotification } from "@/components/backup/backup-notifications";
import { BackupSupportTools } from "@/components/backup/backup-support-tools";
import { newBackupJob, type BackupNotification, type Diagnostics, type LogOutputs } from "@/stores/backup-store";

beforeEach(() => {
  mockInvoke.mockReset();
  mockInvoke.mockResolvedValue(false);
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

describe("notifications editor", () => {
  it("adds channels with the engine's shape and keeps secrets out of the job", async () => {
    let value: BackupNotification[] = [];
    const onChange = vi.fn((v: BackupNotification[]) => (value = v));
    const { rerender } = render(<NotificationsEditor value={value} onChange={onChange} />);
    fireEvent.click(screen.getByTestId("notification-add-email"));
    expect(value[0]).toMatchObject({ type: "email", tls: "start_tls", port: null, to: [], send_on: ["warning", "error"], cert: { accept_invalid_certs: false, ca_pem: null } });
    expect(value[0].credential_id).toMatch(/[0-9a-f-]{36}/);
    rerender(<NotificationsEditor value={value} onChange={onChange} />);
    fireEvent.change(screen.getByPlaceholderText("smtp.example.com"), { target: { value: "mail.local" } });
    expect(value[0]).toMatchObject({ host: "mail.local" });
    rerender(<NotificationsEditor value={value} onChange={onChange} />);
    fireEvent.change(screen.getByLabelText(/To \(comma/), { target: { value: "a@x.org, b@x.org" } });
    expect((value[0] as Extract<BackupNotification, { type: "email" }>).to).toEqual(["a@x.org", "b@x.org"]);
    rerender(<NotificationsEditor value={value} onChange={onChange} />);

    fireEvent.change(screen.getByTestId("notification-secret"), { target: { value: "p4ss" } });
    fireEvent.click(screen.getByTestId("notification-secret-save"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_set_destination_secret", { credentialId: value[0].credential_id, secret: "p4ss" }));
    expect(JSON.stringify(value)).not.toContain("p4ss");

    mockInvoke.mockResolvedValue(null);
    fireEvent.click(screen.getByTestId("notification-test"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_test_notification", { notification: value[0] }));
    expect(await screen.findByText("Test message sent.")).toBeInTheDocument();
  });

  it("telegram and xmpp defaults", () => {
    expect(newNotification("telegram")).toMatchObject({ type: "telegram", chat_id: "", api_base: "" });
    expect(newNotification("xmpp")).toMatchObject({ type: "xmpp", jid: "", to: "", host: "", port: null, tls: "start_tls" });
  });

  it("report extras edit redaction and minimum level", () => {
    const onChange = vi.fn();
    const reports = newBackupJob().reports;
    render(<ReportExtras reports={reports} onChange={onChange} />);
    fireEvent.click(screen.getByLabelText(/Hide file paths/));
    expect(onChange).toHaveBeenLastCalledWith({ ...reports, redact_paths: true });
    fireEvent.change(screen.getByDisplayValue("info"), { target: { value: "warning" } });
    expect(onChange).toHaveBeenLastCalledWith({ ...reports, min_log_level: "warning" });
  });
});

describe("log outputs", () => {
  it("enables a rotating file with tag filters", () => {
    let value: LogOutputs = newBackupJob().log_outputs;
    const onChange = vi.fn((v: LogOutputs) => (value = v));
    const { rerender } = render(<LogOutputsEditor value={value} onChange={onChange} />);
    fireEvent.click(screen.getByLabelText("Write to a log file"));
    expect(value.file).toBe("");
    rerender(<LogOutputsEditor value={value} onChange={onChange} />);
    fireEvent.change(screen.getByTestId("log-file-path"), { target: { value: "/var/log/ufop.log" } });
    rerender(<LogOutputsEditor value={value} onChange={onChange} />);
    fireEvent.change(screen.getByLabelText("Never tags"), { target: { value: "script, replica" } });
    expect(value).toMatchObject({ file: "/var/log/ufop.log", file_max_mb: 10, file_filter: { exclude_tags: ["script", "replica"], min_level: "info" } });
    fireEvent.click(screen.getByLabelText("Write to the system log"));
    expect(value).toMatchObject({ system_log: true, system_filter: { min_level: "warning" } });
  });
});

describe("support tools", () => {
  it("runs diagnostics and shows problems and timings", async () => {
    const diag: Diagnostics = {
      database: {
        info: null,
        problems: [],
        missing_remote: ["ufop-1.dblock.zip.aes"],
        size_mismatch: [],
        untracked_remote: ["ufop-stray.bin"],
        remote_files: 9,
        tracked_volumes: 8,
        check_ms: 12,
      },
      database_error: null,
      network: { host: "s3.example.com", port: 443, tls: true, addresses: ["10.0.0.1"], dns_ms: 1.5, tcp_ms: 2.25, tls_ms: 8, tls_version: "TLSv1_3", error: null },
      throughput: { list_ms: [3, 2, 2], test_bytes: 1048576, upload_ms: 10, upload_bps: 104857600, download_ms: 5, download_bps: 209715200, verified: true, error: null },
    };
    mockInvoke.mockImplementation(async (cmd: string) => (cmd === "backup_diagnose" ? diag : null));
    const job = { ...newBackupJob(), id: "j1" };
    render(<BackupSupportTools job={job} />);
    fireEvent.click(screen.getByTestId("backup-diagnose"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_diagnose", { jobId: "j1", network: true, throughputKb: 1024 }));
    expect(await screen.findByText("ufop-1.dblock.zip.aes")).toBeInTheDocument();
    expect(screen.getByText(/problems found/)).toBeInTheDocument();
    expect(screen.getByText(/TLS 8.0 ms TLSv1_3/)).toBeInTheDocument();
    expect(screen.getByText("ufop-stray.bin")).toBeInTheDocument();
  });

  it("creates a bug report in the chosen folder", async () => {
    mockInvoke.mockImplementation(async (cmd: string, a: { outPath: string }) =>
      cmd === "backup_bug_report" ? { path: a.outPath, size: 2048, files: ["job.json", "database.sqlite"], paths_obfuscated: 4, runs_included: 2 } : null,
    );
    const job = { ...newBackupJob(), id: "j1", name: "My Docs" };
    render(<BackupSupportTools job={job} />);
    expect(screen.getByTestId("backup-bug-report")).toBeDisabled();
    fireEvent.change(screen.getByTestId("bug-report-folder"), { target: { value: "/home/me/" } });
    fireEvent.click(screen.getByTestId("backup-bug-report"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("backup_bug_report", { jobId: "j1", outPath: expect.stringMatching(/^\/home\/me\/ufop-bug-report-My_Docs-\d{4}-\d{2}-\d{2}\.zip$/) }),
    );
    expect(await screen.findByText(/4 paths hashed, 2 runs/)).toBeInTheDocument();
  });
});
