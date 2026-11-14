/**
 * Repository format UI (K72/K83/K3/K49/K82/K59): creation options map onto
 * the Rust `BackupOptions` serde shape, maintenance sends tagged
 * `BackupOperation`s, and the password change calls its IPC command.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useUIStore } from "@/stores/ui-store";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { BackupChangePassphrase, BackupRepositoryFormat, RepositoryFormatOptions } from "@/components/backup/backup-repository-format";
import { BackupMaintenance } from "@/components/backup/backup-maintenance";
import { describeChunker, describeLayout, newBackupJob, type RepositoryFormatInfo } from "@/stores/backup-store";

const V1_INFO: RepositoryFormatInfo = {
  format_version: 1,
  chunker: { type: "fixed", size: 102400 },
  block_codec: "deflate",
  metadata_compression: "none",
  cipher: "aes256_gcm",
  envelope_key: false,
  layout: { type: "flat" },
  features: [],
  rollback_available: false,
};

beforeEach(() => {
  useUIStore.setState({ backupAdvanced: true });
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

describe("creation options", () => {
  it("defaults to format 2 and maps choices onto BackupOptions", () => {
    const o = newBackupJob().options;
    expect(o).toMatchObject({ format_version: null, chunker: null, block_codec: null, metadata_compression: null, cipher: "aes256_gcm", layout: { type: "flat" } });
    const onChange = vi.fn();
    render(<RepositoryFormatOptions options={o} locked={false} onChange={onChange} />);
    fireEvent.change(screen.getByTestId("opt-cipher"), { target: { value: "chacha20_poly1305" } });
    expect(onChange).toHaveBeenLastCalledWith({ cipher: "chacha20_poly1305" });
    fireEvent.change(screen.getByTestId("opt-layout"), { target: { value: "sharded" } });
    expect(onChange).toHaveBeenLastCalledWith({ layout: { type: "sharded", prefix_len: 2 } });
    fireEvent.change(screen.getByTestId("opt-chunker"), { target: { value: "fixed" } });
    expect(onChange).toHaveBeenLastCalledWith({ chunker: "fixed" });
    fireEvent.change(screen.getByTestId("opt-format-version"), { target: { value: "1" } });
    expect(onChange).toHaveBeenLastCalledWith({ format_version: 1, chunker: null, block_codec: null, metadata_compression: null, cipher: "aes256_gcm", layout: { type: "flat" } });
  });

  it("format 1 and existing repositories lock the format 2 choices", () => {
    const o = { ...newBackupJob().options, format_version: 1 };
    const { rerender } = render(<RepositoryFormatOptions options={o} locked={false} onChange={() => {}} />);
    expect(screen.getByTestId("opt-cipher")).toBeDisabled();
    expect(screen.getByTestId("opt-format-version")).not.toBeDisabled();
    rerender(<RepositoryFormatOptions options={newBackupJob().options} locked onChange={() => {}} />);
    expect(screen.getByTestId("opt-format-version")).toBeDisabled();
  });

  it("describes chunkers and layouts", () => {
    expect(describeChunker({ type: "fastcdc", min: 25600, avg: 102400, max: 409600 })).toContain("FastCDC");
    expect(describeLayout({ type: "sharded", prefix_len: 2 })).toContain("2-digit");
  });
});

describe("maintenance format tools", () => {
  it("shows the format, upgrades after confirmation and previews a reshard", async () => {
    const job = { ...newBackupJob(), id: "j1" };
    let info = V1_INFO;
    mockInvoke.mockImplementation(async (cmd: string, args: { operation?: { type: string } }) => {
      if (cmd !== "backup_execute") return [];
      const t = args.operation?.type;
      if (t === "format_info") return { run_id: "r", status: "success", summary: "Repository format", report: info, log: [] };
      if (t === "upgrade_format") {
        info = { ...V1_INFO, format_version: 2, block_codec: "zstd", envelope_key: true, rollback_available: true };
        return { run_id: "r", status: "success", summary: "ok", report: { message: "Upgraded to format 2." }, log: [] };
      }
      if (t === "reshard") return { run_id: "r", status: "success", summary: "moved", report: { moved: 4, bytes_moved: 2048, already_in_place: 0, dry_run: true, errors: [] }, log: [] };
      return { run_id: "r", status: "success", summary: "", report: {}, log: [] };
    });
    render(<BackupRepositoryFormat job={job} />);
    fireEvent.click(screen.getByTestId("backup-format-info"));
    expect(await screen.findByTestId("backup-format-details")).toHaveTextContent("Deflate");
    fireEvent.click(screen.getByTestId("backup-format-upgrade"));
    fireEvent.click(screen.getByTestId("backup-format-upgrade-confirm"));
    await waitFor(() => expect(screen.getByTestId("backup-format-rollback")).toBeInTheDocument());
    expect(mockInvoke).toHaveBeenCalledWith("backup_execute", { id: "j1", operation: { type: "upgrade_format", rollback: false } });
    fireEvent.change(screen.getByTestId("backup-reshard-target"), { target: { value: "1" } });
    fireEvent.click(screen.getByTestId("backup-reshard-preview"));
    expect(await screen.findByTestId("backup-reshard-summary")).toHaveTextContent("4 volume(s)");
    expect(mockInvoke).toHaveBeenCalledWith("backup_execute", { id: "j1", operation: { type: "reshard", layout: { type: "sharded", prefix_len: 1 }, dry_run: true } });
    fireEvent.click(screen.getByTestId("backup-reshard"));
    fireEvent.click(screen.getByTestId("backup-reshard-confirm"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_execute", { id: "j1", operation: { type: "reshard", layout: { type: "sharded", prefix_len: 1 }, dry_run: false } }));
  });

  it("is advanced only in the maintenance view", () => {
    useUIStore.setState({ backupAdvanced: false });
    render(<BackupMaintenance job={{ ...newBackupJob(), id: "j1" }} onDeleted={() => {}} />);
    expect(screen.queryByTestId("backup-format-info")).toBeNull();
  });
});

describe("change password", () => {
  it("requires matching entries and sends the new passphrase", async () => {
    mockInvoke.mockImplementation(async (cmd: string) =>
      cmd === "backup_change_passphrase" ? { from_version: 2, to_version: 2, unchanged: false, features: [], message: "Passphrase changed." } : [],
    );
    render(<BackupChangePassphrase job={{ ...newBackupJob(), id: "j1" }} />);
    fireEvent.click(screen.getByTestId("backup-change-passphrase-open"));
    fireEvent.change(screen.getByTestId("backup-new-passphrase"), { target: { value: "new-secret" } });
    fireEvent.change(screen.getByTestId("backup-repeat-passphrase"), { target: { value: "other" } });
    expect(screen.getByTestId("backup-change-passphrase-confirm")).toBeDisabled();
    fireEvent.change(screen.getByTestId("backup-repeat-passphrase"), { target: { value: "new-secret" } });
    fireEvent.click(screen.getByTestId("backup-change-passphrase-confirm"));
    expect(await screen.findByText("Passphrase changed.")).toBeInTheDocument();
    expect(mockInvoke).toHaveBeenCalledWith("backup_change_passphrase", { id: "j1", current: null, newPassphrase: "new-secret", kdfPreset: null });
  });

  it("is hidden for unencrypted and GPG jobs", () => {
    const { container, rerender } = render(<BackupChangePassphrase job={{ ...newBackupJob(), encrypted: false }} />);
    expect(container).toBeEmptyDOMElement();
    const j = newBackupJob();
    rerender(<BackupChangePassphrase job={{ ...j, options: { ...j.options, encryption_module: "gpg" } }} />);
    expect(container).toBeEmptyDOMElement();
  });
});
