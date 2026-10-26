/**
 * The guided wizards do the real work through the backend (they used to
 * only add an Activity line): sync pairs, connections, transfers, the
 * interrupted-transfer queue, compat renames, reports, peers, drives and
 * backups.
 */
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, act } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));
vi.mock("@tauri-apps/api/path", () => ({
  documentDir: async () => "/home/me/Documents",
  desktopDir: async () => "/home/me/Desktop",
  pictureDir: async () => "/home/me/Pictures",
  audioDir: async () => "/home/me/Music",
  videoDir: async () => "/home/me/Videos",
  downloadDir: async () => "/home/me/Downloads",
}));
const pickFolderMock = vi.fn(async () => "/home/me/Picked");
vi.mock("@/lib/folder-picker", () => ({ pickFolder: (...a: unknown[]) => pickFolderMock(...(a as [])) }));

import { ConnectLocalSyncWizard, ConnectSFTPWizard, ConnectSMBWizard } from "@/components/guided-flows/connection-flows";
import { CreateSyncWizard, HandleCompatWarningWizard, ResolveInterruptionWizard, StartTransferWizard } from "@/components/guided-flows/operation-flows";
import {
  BackupFolderWizard,
  CopyToExternalWizard,
  ExportReportWizard,
  MigrateComputersWizard,
  OldToNewDriveWizard,
  RestoreNamesWizard,
  TransferToComputerWizard,
} from "@/components/guided-flows/utility-flows";
import { getDefaultCommands } from "@/components/command-palette";
import { syncTrigger } from "@/lib/guided-actions";
import { useUIStore } from "@/stores/ui-store";

type Handler = (a: Record<string, unknown>) => unknown;
const calls = (cmd: string) => mockInvoke.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a as Record<string, unknown>);
let over: Record<string, Handler> = {};

const DIRS = new Set(["/src", "/dst", "/home/me/Docs", "/backup", "/media/old", "/media/new"]);

function backend() {
  mockInvoke.mockImplementation(async (cmd: string, args: Record<string, unknown> = {}) => {
    if (over[cmd]) return over[cmd](args);
    switch (cmd) {
      case "get_file_metadata": {
        const p = String(args.path);
        if (p === "/missing") throw { message: "No such file or directory", advice: "" };
        return { path: p, name: p.split("/").pop(), is_dir: DIRS.has(p) || !p.includes("."), size: 1 };
      }
      case "create_sync_pair":
        return { id: "p1", name: args.name };
      case "list_sync_pairs":
        return [];
      case "save_connection":
        return { id: "c1", name: args.name, protocol: args.protocol };
      case "copy_files":
      case "move_files":
        return { success: true, operation: cmd, source_paths: args.sourcePaths, dest_paths: (args.sourcePaths as string[]).map((s) => `${args.destDir}/${s.split("/").pop()}`), undo_id: "u" };
      case "integrity_checksum":
        return { path: args.path, algorithm: "sha256", hash: "same", size: 1, computed_at: "" };
      case "compare_folders":
        return { summary: { left_only: { files: 0, folders: 0 }, different: { files: 0, folders: 0 }, error: { files: 0, folders: 0 } } };
      case "list_transfers":
        return [
          { id: "j1", source_path: "/a/big.iso", dest_path: "/b/big.iso", total_bytes: 100, transferred_bytes: 40, status: "paused", error_message: null },
          { id: "j2", source_path: "/a/x.zip", dest_path: "/b/x.zip", total_bytes: 10, transferred_bytes: 0, status: "failed", error_message: "Network dropped" },
          { id: "j3", source_path: "/a/done", dest_path: "/b/done", total_bytes: 1, transferred_bytes: 1, status: "completed", error_message: null },
        ];
      case "retry_transfer":
        return true;
      case "compat_profiles":
        return [{ id: "windows-ntfs", name: "Windows NTFS", description: "" }, { id: "s3", name: "Amazon S3", description: "" }];
      case "compat_scan_folder":
        return {
          scanned: 12,
          truncated: false,
          issues: [
            { path: "/home/me/Docs/a:b.txt", name: "a:b.txt", suggested_name: "a_b.txt", rules: ["Forbidden character ':'"], tier: 2, fixable: true },
            { path: "/home/me/Docs/A.txt", name: "A.txt", suggested_name: "A.txt", rules: ["Case collision"], tier: 3, fixable: false },
          ],
        };
      case "compat_fix_names":
        return { renamed: [["/home/me/Docs/a:b.txt", "/home/me/Docs/a_b.txt"]], failed: [] };
      case "compat_list_renames":
        return [
          { id: "r1", original_name: "a:b.txt", translated_name: "a_b.txt", folder: "/home/me/Docs", exists: true, restorable: true, timestamp: "" },
          { id: "r2", original_name: "old.txt", translated_name: "new.txt", folder: "/x", exists: false, restorable: false, timestamp: "" },
        ];
      case "compat_restore_names":
        return { renamed: [["/home/me/Docs/a_b.txt", "/home/me/Docs/a:b.txt"]], failed: [] };
      case "pick_save_file":
        return "/home/me/report.csv";
      case "export_transfer_history":
        return "id,status\n1,completed\n";
      case "peer_list_online":
        return [{ id: "11111111-1111-4111-8111-111111111111", hostname: "laptop.local", display_name: "Ada's laptop", status: "online" }];
      case "detect_drives":
        return [
          { name: "System", mount_point: "/", total_bytes: 1e12, free_bytes: 5e11, drive_type: "internal" },
          { name: "USB", mount_point: "/media/usb", total_bytes: 64e9, free_bytes: 60e9, drive_type: "external" },
          { name: "Old", mount_point: "/media/old", total_bytes: 5e11, free_bytes: 1e10, drive_type: "external" },
          { name: "New", mount_point: "/media/new", total_bytes: 2e12, free_bytes: 2e12, drive_type: "external" },
        ];
      case "count_affected_files":
        return { files: 3, bytes: 3000, capped: false };
      case "transfer_preflight":
        return { can_proceed: true, warnings: [], errors: [] };
      case "backup_save_job":
        return { ...(args.job as object), id: "b1" };
      case "backup_run":
        return "task1";
      default:
        return undefined;
    }
  });
}

beforeAll(() => {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

beforeEach(() => {
  over = {};
  useUIStore.setState({ activeGuidedFlow: "x", activityFeed: [] });
  mockInvoke.mockReset();
  pickFolderMock.mockClear();
  backend();
});

/** Click the wizard's main button (Next, or the finish label). */
const next = (name: RegExp = /^(Next|Finish)$/) => fireEvent.click(screen.getByRole("button", { name }));
const type = (placeholder: string, value: string, index = 0) => fireEvent.change(screen.getAllByPlaceholderText(placeholder)[index], { target: { value } });
const closed = () => waitFor(() => expect(useUIStore.getState().activeGuidedFlow).toBeNull());
const lastActivity = () => useUIStore.getState().activityFeed[0]?.summary ?? "";

describe("sync wizards", () => {
  it("Set Up Local Sync tests the folders for real and creates a two-way pair", async () => {
    render(<ConnectLocalSyncWizard />);
    type("/Users/you/Documents", "/src");
    next();
    type("/Volumes/Backup/Documents", "/missing");
    next();
    next();
    fireEvent.click(screen.getByTestId("wizard-test-folders"));
    expect(await screen.findByRole("alert")).toHaveTextContent('"/missing" cannot be opened');
    // Finishing with a missing folder fails and keeps the wizard open.
    next(/Create Sync Pair/);
    await waitFor(() => expect(screen.getAllByRole("alert").length).toBeGreaterThan(0));
    expect(calls("create_sync_pair")).toHaveLength(0);
    expect(useUIStore.getState().activeGuidedFlow).toBe("x");
  });

  it("Set Up Local Sync saves the pair through the sync store", async () => {
    render(<ConnectLocalSyncWizard />);
    type("/Users/you/Documents", "/src");
    next();
    type("/Volumes/Backup/Documents", "/dst");
    next();
    next();
    fireEvent.click(screen.getByTestId("wizard-test-folders"));
    await screen.findByText("Both folders are accessible");
    next(/Create Sync Pair/);
    await closed();
    expect(calls("create_sync_pair")[0]).toMatchObject({ sourcePath: "/src", destPath: "/dst", mode: "two_way", trigger: "manual", name: "src → dst" });
  });

  it("Create Sync Pair turns the schedule into a trigger", async () => {
    render(<CreateSyncWizard />);
    type("e.g., Documents Backup, Work Files Sync", "Docs");
    next();
    type("/Users/you/Documents", "/src");
    type("/Volumes/Backup/Documents", "/dst");
    next();
    fireEvent.click(screen.getByText("Every hour"));
    next();
    next(/Create Sync/);
    await closed();
    expect(calls("create_sync_pair")[0]).toMatchObject({ name: "Docs", mode: "one_way", trigger: "scheduled", cronExpr: "0 0 * * * * *" });
    expect(syncTrigger("daily")).toEqual({ trigger: "scheduled", cronExpr: "0 0 2 * * * *" });
    expect(syncTrigger("realtime")).toEqual({ trigger: "watch" });
  });
});

describe("server wizards", () => {
  it("Windows share: the test saves the connection, signs in and lists the share; a failure is shown", async () => {
    over.connector_list_remote = () => {
      throw { message: "Access denied to SharedFiles", advice: "" };
    };
    render(<ConnectSMBWizard />);
    type("office-nas or 192.168.1.100", "nas");
    next();
    type("SharedFiles", "SharedFiles");
    next();
    type("optional", "ada", 0);
    type("optional", "pw", 1);
    next();
    fireEvent.click(screen.getByTestId("wizard-test-connection"));
    expect(await screen.findByRole("alert")).toHaveTextContent("Access denied to SharedFiles");
    expect(calls("save_connection")[0]).toMatchObject({ protocol: "smb", host: "nas", username: "ada", password: "pw", remotePath: "/SharedFiles", connectionId: null });
    expect(calls("connector_connect")[0]).toMatchObject({ protocol: "smb", connectionId: "c1" });
    // Fixed and tried again: the same connection is updated.
    over = {};
    next(/^Connect$/);
    await closed();
    expect(calls("save_connection")[1]).toMatchObject({ connectionId: "c1" });
    expect(lastActivity()).toContain("Connection Manager");
  });

  // Review finding: a passed test stayed passed after the host changed, so
  // Finish kept the old server while the summary named the new one.
  it("changing a field after a passed test tests again on Finish", async () => {
    render(<ConnectSMBWizard />);
    type("office-nas or 192.168.1.100", "nas-a");
    next();
    type("SharedFiles", "S");
    next();
    next();
    fireEvent.click(screen.getByTestId("wizard-test-connection"));
    await screen.findByText("Connected successfully");
    fireEvent.click(screen.getByRole("button", { name: /Back/ }));
    fireEvent.click(screen.getByRole("button", { name: /Back/ }));
    fireEvent.click(screen.getByRole("button", { name: /Back/ }));
    type("office-nas or 192.168.1.100", "nas-b");
    next();
    next();
    next();
    next(/^Connect$/);
    await closed();
    expect(calls("save_connection").map((a) => a.host)).toEqual(["nas-a", "nas-b"]);
    expect(lastActivity()).toContain("nas-b");
  });

  it("cancelling removes the connection a test saved", async () => {
    render(<ConnectSMBWizard />);
    type("office-nas or 192.168.1.100", "nas");
    next();
    type("SharedFiles", "S");
    next();
    next();
    fireEvent.click(screen.getByTestId("wizard-test-connection"));
    await screen.findByText("Connected successfully");
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await closed();
    expect(calls("delete_connection")).toEqual([{ connectionId: "c1" }]);
  });

  it("SFTP with an SSH key sends no password", async () => {
    render(<ConnectSFTPWizard />);
    type("server.example.com", "srv");
    type("your-username", "ada");
    next();
    fireEvent.click(screen.getByText("SSH Key"));
    next();
    next();
    next(/^Connect$/);
    await closed();
    expect(calls("save_connection")[0]).toMatchObject({ protocol: "sftp", host: "srv", port: 22, username: "ada", password: null });
    expect(calls("connector_list_remote")[0]).toMatchObject({ protocol: "sftp", path: "/" });
  });
});

describe("transfer wizards", () => {
  it("Transfer Files copies and verifies by checksum; a mismatch keeps the wizard open", async () => {
    over.integrity_checksum = (a) => ({ hash: String(a.path).startsWith("/dst") ? "other" : "orig" });
    render(<StartTransferWizard />);
    type("/Users/you/Documents/project", "/src/file.txt");
    next();
    type("/Volumes/External/backup", "/dst");
    next();
    fireEvent.click(screen.getByText("Verify files after transfer"));
    next();
    next(/Start Transfer/);
    expect(await screen.findByText(/the check found a problem: The copy's content differs/)).toBeInTheDocument();
    expect(calls("copy_files")[0]).toEqual({ sourcePaths: ["/src/file.txt"], destDir: "/dst" });
    expect(calls("integrity_checksum").map((a) => a.path)).toEqual(["/src/file.txt", "/dst/file.txt"]);
    expect(useUIStore.getState().activeGuidedFlow).toBe("x");
  });

  it("Interrupted Operation lists paused and failed transfers and resumes them", async () => {
    render(<ResolveInterruptionWizard />);
    expect(await screen.findAllByTestId("wizard-interrupted-job")).toHaveLength(2);
    expect(screen.getByText(/Network dropped/)).toBeInTheDocument();
    next();
    next();
    next(/Apply/);
    await closed();
    expect(calls("resume_transfer")).toEqual([{ jobId: "j1" }]);
    expect(calls("retry_transfer")).toEqual([{ jobId: "j2" }]);
  });

  it("Interrupted Operation can cancel, and says when nothing waits", async () => {
    const { unmount } = render(<ResolveInterruptionWizard />);
    await screen.findAllByTestId("wizard-interrupted-job");
    next();
    fireEvent.click(screen.getByText("Cancel", { selector: "span" }));
    next();
    next(/Apply/);
    await closed();
    expect(calls("cancel_transfer").map((a) => a.jobId)).toEqual(["j1", "j2"]);
    unmount();
    over.list_transfers = () => [];
    useUIStore.setState({ activeGuidedFlow: "x" });
    render(<ResolveInterruptionWizard />);
    expect(await screen.findByTestId("wizard-no-interrupted")).toBeInTheDocument();
  });
});

describe("file name wizards", () => {
  it("Fix File Name Issues scans for the destination and renames only what can be fixed", async () => {
    render(<HandleCompatWarningWizard />);
    fireEvent.change(screen.getByTestId("wizard-compat-folder"), { target: { value: "/home/me/Docs" } });
    await waitFor(() => expect(screen.getByRole("option", { name: "Amazon S3" })).toBeInTheDocument());
    next();
    expect(await screen.findAllByTestId("wizard-compat-issue")).toHaveLength(2);
    expect(calls("compat_scan_folder")[0]).toEqual({ path: "/home/me/Docs", profileId: "windows-ntfs" });
    expect(screen.getByText(/needs a manual rename/)).toBeInTheDocument();
    next();
    next(/Rename Files/);
    await closed();
    expect(calls("compat_fix_names")[0]).toEqual({ fixes: [{ path: "/home/me/Docs/a:b.txt", new_name: "a_b.txt" }], profileId: "windows-ntfs" });
  });

  it("Restore Original Names lists the recorded renames and restores the chosen ones", async () => {
    render(<RestoreNamesWizard />);
    const rows = await screen.findAllByTestId("wizard-rename-record");
    expect(rows[1]).toBeDisabled();
    fireEvent.click(rows[0]);
    next();
    next();
    next(/Restore Names/);
    await closed();
    expect(calls("compat_restore_names")[0]).toEqual({ ids: ["r1"] });
  });
});

describe("report, peer, drive and backup wizards", () => {
  it("Export Report saves the transfer history where the user chose; cancelling stays open", async () => {
    render(<ExportReportWizard />);
    next();
    next();
    next(/Save Report/);
    await closed();
    expect(calls("export_transfer_history")[0]).toMatchObject({ format: "csv" });
    expect(calls("write_text_file_full")[0]).toEqual({ path: "/home/me/report.csv", content: "id,status\n1,completed\n" });
    expect(calls("pick_save_file")[0]).toMatchObject({ defaultName: expect.stringMatching(/^ufop-transfer-report-\d{4}-\d{2}-\d{2}\.csv$/) });
  });

  it("Export Report: a cancelled save dialog writes nothing", async () => {
    over.pick_save_file = () => null;
    render(<ExportReportWizard />);
    next();
    next();
    next(/Save Report/);
    await waitFor(() => expect(calls("pick_save_file")).toHaveLength(1));
    expect(calls("write_text_file_full")).toHaveLength(0);
    expect(useUIStore.getState().activeGuidedFlow).toBe("x");
  });

  it("Send to Another Computer offers the chosen folders to the chosen peer", async () => {
    render(<TransferToComputerWizard />);
    fireEvent.click(await screen.findByTestId("wizard-peer"));
    next();
    fireEvent.click(screen.getByTestId("wizard-add-folder"));
    await screen.findByText("/home/me/Picked");
    next();
    next(/^Send$/);
    await closed();
    expect(calls("peer_request_transfer")[0]).toEqual({ peerId: "11111111-1111-4111-8111-111111111111", paths: ["/home/me/Picked"] });
  });

  it("Move to a New Computer sends the user's folders", async () => {
    render(<MigrateComputersWizard />);
    fireEvent.click(await screen.findByTestId("wizard-peer"));
    next();
    fireEvent.click(screen.getByTestId("wizard-category-Desktop"));
    next();
    next(/^Send$/);
    await closed();
    expect(calls("peer_request_transfer")[0]).toMatchObject({ paths: ["/home/me/Documents", "/home/me/Pictures"] });
  });

  it("Copy to External lists external drives only and refuses when there is no room", async () => {
    over.transfer_preflight = () => ({ can_proceed: false, warnings: [], errors: [{ code: "space", message: "Needs 3 KB, 1 KB free", detail: null }] });
    render(<CopyToExternalWizard />);
    fireEvent.change(screen.getByTestId("wizard-source"), { target: { value: "/home/me/Docs" } });
    next();
    const drives = await screen.findAllByTestId("wizard-drive");
    expect(drives.map((d) => d.textContent)).toEqual([expect.stringContaining("USB"), expect.stringContaining("Old"), expect.stringContaining("New")]);
    fireEvent.click(drives[0]);
    next();
    next(/Start Copy/);
    expect(await screen.findByText("Needs 3 KB, 1 KB free")).toBeInTheDocument();
    expect(calls("copy_files")).toHaveLength(0);
  });

  it("Move to a New Drive copies into a folder of its own and checks every file", async () => {
    render(<OldToNewDriveWizard />);
    fireEvent.click((await screen.findAllByTestId("wizard-drive"))[2]); // Old
    next();
    const targets = await screen.findAllByTestId("wizard-drive");
    expect(targets.some((d) => d.textContent?.includes("/media/old"))).toBe(false);
    fireEvent.click(targets.find((d) => d.textContent?.includes("New"))!);
    next();
    next();
    next(/Start Copy/);
    await closed();
    expect(calls("ensure_directory")[0]).toEqual({ path: "/media/new/From old" });
    expect(calls("copy_files")[0]).toEqual({ sourcePaths: ["/media/old"], destDir: "/media/new/From old" });
    expect(calls("compare_folders")[0]).toMatchObject({ left: { type: "local", path: "/media/old" }, right: { type: "local", path: "/media/new/From old/old" }, options: { compare_mode: "checksum" } });
    expect(lastActivity()).toContain("Every file was checked");
  });

  it("Back Up a Folder makes an encrypted daily job and runs it", async () => {
    render(<BackupFolderWizard />);
    fireEvent.change(screen.getByTestId("wizard-backup-source"), { target: { value: "/home/me/Docs" } });
    next();
    fireEvent.change(screen.getByTestId("wizard-backup-dest"), { target: { value: "/backup" } });
    next();
    fireEvent.change(screen.getByTestId("wizard-backup-passphrase"), { target: { value: "long secret" } });
    fireEvent.change(screen.getByTestId("wizard-backup-repeat"), { target: { value: "long secrex" } });
    next();
    expect(await screen.findByText("The two passphrases are different.")).toBeInTheDocument();
    fireEvent.change(screen.getByTestId("wizard-backup-repeat"), { target: { value: "long secret" } });
    next();
    fireEvent.click(screen.getByTestId("wizard-backup-daily"));
    await act(async () => next(/Start Backup/));
    await closed();
    const saved = calls("backup_save_job")[0];
    expect(saved.passphrase).toBe("long secret");
    expect(saved.job).toMatchObject({ name: "Backup of Docs", sources: ["/home/me/Docs"], destination: { type: "local", path: "/backup" }, encrypted: true, schedule: { enabled: true, interval_minutes: 1440 } });
    expect(calls("backup_run")[0]).toEqual({ id: "b1", dryRun: false });
  });
});

describe("reachable from the command palette", () => {
  it("lists every guided wizard and opens it", () => {
    const open = vi.fn();
    const cmds = getDefaultCommands({ onOpenGuidedFlow: open });
    const guided = cmds.filter((c) => c.category === "Guided");
    expect(guided).toHaveLength(18);
    guided.find((c) => c.id === "guided-backup-folder")!.action();
    expect(open).toHaveBeenCalledWith("backup-folder");
  });
});
