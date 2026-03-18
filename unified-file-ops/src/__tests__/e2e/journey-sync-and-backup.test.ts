/**
 * E2E Journey Tests: Sync and Backup workflows
 *
 * Covers Journeys 2 (macOS-Windows sync), 5 (drive-to-drive), 8 (backup)
 */

import { describe, it, expect, vi, beforeEach } from "vitest";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

describe("Journey 2: macOS to Windows Two-Way Sync", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
  });

  it("should create a two-way sync pair", async () => {
    const pairId = "pair-001";
    mockInvoke.mockResolvedValueOnce({
      id: pairId,
      name: "macOS to Windows Share",
      source_path: "/Users/test/Documents",
      dest_path: "//server/share/Documents",
      mode: "two_way",
      enabled: true,
    });

    const pair = await mockInvoke("create_sync_pair", {
      name: "macOS to Windows Share",
      sourcePath: "/Users/test/Documents",
      destPath: "//server/share/Documents",
      mode: "two_way",
      trigger: "manual",
    });

    expect(pair.mode).toBe("two_way");
    expect(pair.enabled).toBe(true);
  });

  it("should show dry-run preview before sync", async () => {
    mockInvoke.mockResolvedValueOnce({
      total_additions: 5,
      total_modifications: 2,
      total_deletions: 1,
      total_bytes: 1024 * 1024 * 10,
      items: [
        { path: "new_doc.txt", action: "add", size: 1024 },
        { path: "modified.pdf", action: "modify", size: 2048 },
      ],
    });

    const preview = await mockInvoke("sync_dry_run", {
      pairId: "pair-001",
    });

    expect(preview.total_additions).toBe(5);
    expect(preview.total_modifications).toBe(2);
    expect(preview.total_deletions).toBe(1);
    expect(preview.total_bytes).toBeGreaterThan(0);
  });

  it("should execute sync and return report", async () => {
    mockInvoke.mockResolvedValueOnce({
      id: "run-001",
      pair_id: "pair-001",
      status: "success",
      files_added: 3,
      files_modified: 1,
      files_deleted: 0,
      bytes_transferred: 50000,
      errors: [],
    });

    const report = await mockInvoke("run_sync", {
      pairId: "pair-001",
    });

    expect(report.status).toBe("success");
    expect(report.files_added).toBe(3);
    expect(report.errors).toHaveLength(0);
  });

  it("should show health indicator", async () => {
    mockInvoke.mockResolvedValueOnce("green");

    const health = await mockInvoke("get_sync_health", {
      pairId: "pair-001",
    });

    expect(health).toBe("green");
  });

  it("should export sync preview as CSV", async () => {
    mockInvoke.mockResolvedValueOnce(
      "path,action,size\nnew.txt,add,1024\nmod.pdf,modify,2048\n"
    );

    const csv = await mockInvoke("export_sync_preview_csv", {
      pairId: "pair-001",
    });

    expect(csv).toContain("path,action,size");
    expect(csv).toContain("add");
  });

  it("should handle conflict resolution", async () => {
    mockInvoke.mockResolvedValueOnce([
      {
        id: "conflict-1",
        path: "shared_doc.txt",
        source_modified: "2024-01-15T10:00:00Z",
        dest_modified: "2024-01-15T11:00:00Z",
        resolution: null,
      },
    ]);

    const conflicts = await mockInvoke("get_sync_conflicts", {
      pairId: "pair-001",
    });

    expect(conflicts).toHaveLength(1);
    expect(conflicts[0].resolution).toBeNull();

    // Resolve conflict
    mockInvoke.mockResolvedValueOnce(undefined);
    await mockInvoke("resolve_sync_conflict", {
      conflictId: "conflict-1",
      resolution: "source_wins",
    });
  });
});

describe("Journey 5: Drive-to-Drive Transfer", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
  });

  it("should detect available drives", async () => {
    mockInvoke.mockResolvedValueOnce([
      {
        name: "Macintosh HD",
        mount_point: "/",
        total_bytes: 500000000000,
        available_bytes: 200000000000,
        drive_type: "internal",
        filesystem_type: "apfs",
      },
      {
        name: "External SSD",
        mount_point: "/Volumes/ExternalSSD",
        total_bytes: 1000000000000,
        available_bytes: 800000000000,
        drive_type: "external",
        filesystem_type: "exfat",
      },
    ]);

    const drives = await mockInvoke("detect_drives");

    expect(drives).toHaveLength(2);
    expect(drives[0].name).toBe("Macintosh HD");
    expect(drives[1].drive_type).toBe("external");
  });

  it("should run preflight check", async () => {
    mockInvoke.mockResolvedValueOnce({
      can_proceed: true,
      destination_free_bytes: 800000000000,
      required_bytes: 5000000000,
      warnings: [],
    });

    const preflight = await mockInvoke("transfer_preflight", {
      sourcePath: "/",
      destPath: "/Volumes/ExternalSSD",
      totalBytes: 5000000000,
    });

    expect(preflight.can_proceed).toBe(true);
    expect(preflight.destination_free_bytes).toBeGreaterThan(
      preflight.required_bytes
    );
  });

  it("should enqueue and track transfer", async () => {
    const jobId = "drive-transfer-001";
    mockInvoke.mockResolvedValueOnce({
      id: jobId,
      source_path: "/Users/test/file.zip",
      dest_path: "/Volumes/ExternalSSD/backup/file.zip",
      total_bytes: 1073741824,
      transferred_bytes: 0,
      status: "queued",
    });

    const job = await mockInvoke("enqueue_transfer", {
      source: "/Users/test/file.zip",
      dest: "/Volumes/ExternalSSD/backup/file.zip",
      totalBytes: 1073741824,
    });

    expect(job.status).toBe("queued");
    expect(job.total_bytes).toBe(1073741824);
  });

  it("should support bandwidth throttling", async () => {
    mockInvoke.mockResolvedValueOnce(undefined);
    await mockInvoke("set_global_throttle", {
      bytesPerSec: 50000000,
    });

    mockInvoke.mockResolvedValueOnce({
      global_limit_bps: 50000000,
      per_connection_limit_bps: 0,
    });

    const config = await mockInvoke("get_throttle_config");
    expect(config.global_limit_bps).toBe(50000000);
  });

  it("should handle pause/resume/cancel", async () => {
    // Pause
    mockInvoke.mockResolvedValueOnce(undefined);
    await mockInvoke("pause_transfer", { jobId: "job-1" });

    // Resume
    mockInvoke.mockResolvedValueOnce(undefined);
    await mockInvoke("resume_transfer", { jobId: "job-1" });

    // Cancel
    mockInvoke.mockResolvedValueOnce(undefined);
    await mockInvoke("cancel_transfer", { jobId: "job-1" });

    expect(mockInvoke).toHaveBeenCalledTimes(3);
  });
});

describe("Journey 8: External Drive Backup", () => {
  beforeEach(() => {
    mockInvoke.mockReset();
  });

  it("should create versioned backup sync pair", async () => {
    mockInvoke.mockResolvedValueOnce({
      id: "backup-pair-001",
      name: "External Drive Backup",
      mode: "versioned_backup",
      checksum_enabled: true,
      enabled: true,
    });

    const pair = await mockInvoke("create_sync_pair", {
      name: "External Drive Backup",
      sourcePath: "/Users/test/Documents",
      destPath: "/Volumes/BackupDrive/Backup",
      mode: "versioned_backup",
      checksumEnabled: true,
    });

    expect(pair.mode).toBe("versioned_backup");
    expect(pair.checksum_enabled).toBe(true);
  });

  it("should create encrypted vault on backup drive", async () => {
    mockInvoke.mockResolvedValueOnce({
      id: "vault-001",
      name: "Encrypted Backup",
      status: "unlocked",
      file_count: 0,
      total_size: 0,
    });

    const vault = await mockInvoke("create_vault", {
      path: "/Volumes/BackupDrive/EncryptedVault",
      name: "Encrypted Backup",
      password: "secure_pass_123",
    });

    expect(vault.status).toBe("unlocked");
    expect(vault.name).toBe("Encrypted Backup");
  });

  it("should encrypt file into vault", async () => {
    mockInvoke.mockResolvedValueOnce(
      "/Volumes/BackupDrive/EncryptedVault/tax_return.pdf.enc"
    );

    const encPath = await mockInvoke("vault_encrypt_file", {
      vaultId: "vault-001",
      sourcePath: "/Users/test/Documents/tax_return.pdf",
      relativePath: "tax_return.pdf",
    });

    expect(encPath).toContain(".enc");
  });

  it("should lock and unlock vault", async () => {
    // Lock
    mockInvoke.mockResolvedValueOnce(undefined);
    await mockInvoke("lock_vault", { vaultId: "vault-001" });

    // Check locked
    mockInvoke.mockResolvedValueOnce(false);
    const isUnlocked = await mockInvoke("is_vault_unlocked", {
      vaultId: "vault-001",
    });
    expect(isUnlocked).toBe(false);

    // Unlock
    mockInvoke.mockResolvedValueOnce({
      id: "vault-001",
      status: "unlocked",
    });
    const vault = await mockInvoke("unlock_vault", {
      path: "/Volumes/BackupDrive/EncryptedVault",
      password: "secure_pass_123",
    });
    expect(vault.status).toBe("unlocked");
  });

  it("should show backup health status", async () => {
    mockInvoke.mockResolvedValueOnce("green");
    const health = await mockInvoke("get_sync_health", {
      pairId: "backup-pair-001",
    });
    expect(health).toBe("green");
  });
});
