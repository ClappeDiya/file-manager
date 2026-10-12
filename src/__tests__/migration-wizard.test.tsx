import { describe, expect, it, vi, beforeEach, afterEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import {
  MigrationWizard,
  type MigrationFolder,
  type MigrationProgress,
} from "@/components/migration-wizard";
import { PromptDialogHost } from "@/components/prompt-dialog-host";
import { DeviceSidebar } from "@/components/device-sidebar";

const mockInvoke = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

const DRIVE_A = {
  id: "/Volumes/A",
  name: "Drive A",
  type: "drive" as const,
  mountPoint: "/Volumes/A",
  totalBytes: 1_000_000_000,
  freeBytes: 500_000_000,
  filesystem: "apfs",
};

const DRIVE_B = {
  id: "/Volumes/B",
  name: "Drive B",
  type: "drive" as const,
  mountPoint: "/Volumes/B",
  totalBytes: 2_000_000_000,
  freeBytes: 1_500_000_000,
  filesystem: "exfat",
};

const FOLDERS: MigrationFolder[] = [
  {
    path: "/Volumes/A/Documents",
    name: "Documents",
    size: 4096,
    fileCount: 3,
    selected: false,
  },
];

const PROGRESS_COMPLETE: MigrationProgress = {
  currentFile: "",
  filesCompleted: 1,
  filesTotal: 1,
  bytesTransferred: 4096,
  bytesTotal: 4096,
  speedBps: 0,
  etaSeconds: null,
  failedCount: 0,
  phase: "complete",
};

function renderWizard(overrides: Record<string, unknown> = {}) {
  const onClose = vi.fn();
  const props = {
    open: true,
    onClose,
    availableDrives: [DRIVE_A, DRIVE_B],
    onScanFolders: vi.fn(async () => FOLDERS),
    onAnalyzeSpace: vi.fn(async () => ({
      totalSelectedBytes: 4096,
      destFreeBytes: 1_500_000_000,
      destTotalBytes: 2_000_000_000,
      hasEnoughSpace: true,
      spaceAfterTransfer: 1_500_000_000 - 4096,
      duplicateBytes: 0,
      duplicateCount: 0,
      compatIssues: [],
    })),
    onExecute: vi.fn(async () => {}),
    onGetProgress: vi.fn(async () => PROGRESS_COMPLETE),
    onGetSummary: vi.fn(async () => ({
      totalFiles: 1,
      totalBytes: 4096,
      filesTransferred: 1,
      bytesTransferred: 4096,
      filesFailed: 0,
      filesSkipped: 0,
      durationMs: 10,
      averageSpeedBps: 0,
      errors: [],
      warnings: [],
      verificationPassed: true,
      resumed: false,
    })),
    ...overrides,
  };
  render(
    <>
      <PromptDialogHost />
      <MigrationWizard {...props} />
    </>,
  );
  return { onClose, props };
}

beforeEach(() => {
  mockInvoke.mockReset();
  // Simulate the Tauri webview so tauriInvoke paths execute.
  (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {};
});

afterEach(() => {
  delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
});

describe("MigrationWizard drive flow", () => {
  it("scans folders for drive migration, requires a destination, and completes the journey", async () => {
    const { props } = renderWizard();

    // Pick the Drive Migration wizard type.
    fireEvent.click(screen.getByText("Drive Migration"));

    // Source drive — before the fix this jumped to select-folders with an
    // empty list and no destination, wedging the flow forever.
    // Every drive renders in both the source and destination lists —
    // the source section comes first in the DOM.
    fireEvent.click(screen.getAllByText("Drive A")[0]);
    await waitFor(() =>
      expect(props.onScanFolders).toHaveBeenCalledWith("/Volumes/A"),
    );

    // Still on connect-source until a destination is picked — the
    // destination list must remain visible.
    expect(screen.getByText("Select Destination Drive")).toBeInTheDocument();
    fireEvent.click(screen.getAllByText("Drive B")[1]);

    // Now in select-folders with the scanned folder list.
    await waitFor(() =>
      expect(screen.getByText("Documents")).toBeInTheDocument(),
    );

    // Select the folder → Analyze becomes available.
    fireEvent.click(screen.getByRole("checkbox"));
    // "Analyze"/"Preview" are also step-indicator labels — target the
    // footer button by role.
    const analyze = await screen.findByRole("button", { name: "Analyze" });
    fireEvent.click(analyze);
    await waitFor(() =>
      expect(props.onAnalyzeSpace).toHaveBeenCalledWith(
        "/Volumes/A",
        "/Volumes/B",
        ["/Volumes/A/Documents"],
      ),
    );

    // Analysis → Preview → Start Migration.
    fireEvent.click(await screen.findByRole("button", { name: "Preview" }));
    fireEvent.click(await screen.findByRole("button", { name: "Start Migration" }));

    await waitFor(() =>
      expect(props.onExecute).toHaveBeenCalledWith(
        expect.objectContaining({
          type: "drive",
          sourceId: "/Volumes/A",
          destId: "/Volumes/B",
          folders: ["/Volumes/A/Documents"],
        }),
      ),
    );

    // Progress reports complete → summary renders.
    expect(
      await screen.findByRole("button", { name: "Done" }),
    ).toBeInTheDocument();
    expect(props.onGetSummary).toHaveBeenCalled();
  });

  it("confirm dialog appears when closing during an active transfer", async () => {
    const transferring: MigrationProgress = {
      ...PROGRESS_COMPLETE,
      phase: "transferring",
      filesCompleted: 0,
      bytesTransferred: 0,
    };
    const { onClose, props } = renderWizard({
      onGetProgress: vi.fn(async () => transferring),
    });

    fireEvent.click(screen.getByText("Drive Migration"));
    fireEvent.click(screen.getAllByText("Drive A")[0]);
    await waitFor(() =>
      expect(props.onScanFolders).toHaveBeenCalledWith("/Volumes/A"),
    );
    fireEvent.click(screen.getAllByText("Drive B")[1]);
    await waitFor(() => screen.getByText("Documents"));
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(await screen.findByRole("button", { name: "Analyze" }));
    fireEvent.click(await screen.findByRole("button", { name: "Preview" }));
    fireEvent.click(await screen.findByRole("button", { name: "Start Migration" }));

    // Transferring progress is live → close asks for confirmation inside
    // the app (window.confirm is dead in the Tauri webview).
    await waitFor(() => screen.getByLabelText("Close wizard"));
    fireEvent.click(screen.getByLabelText("Close wizard"));

    expect(
      await screen.findByText(
        "A transfer is in progress. Are you sure you want to close?",
      ),
    ).toBeInTheDocument();
    expect(onClose).not.toHaveBeenCalled();

    fireEvent.click(screen.getByText("OK"));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });
});

describe("DeviceSidebar preflight", () => {
  it("passes totalBytes to transfer_preflight", async () => {
    const drives = [
      {
        name: "Macintosh HD",
        mount_point: "/",
        device: "disk1",
        total_bytes: 1_000_000,
        free_bytes: 400_000,
        used_bytes: 600_000,
        filesystem: "apfs",
        drive_type: "internal",
        removable: false,
        read_only: false,
        label: null,
        uuid: null,
      },
      {
        name: "USB Stick",
        mount_point: "/Volumes/USB",
        device: "disk2",
        total_bytes: 500_000,
        free_bytes: 499_000,
        used_bytes: 1_000,
        filesystem: "exfat",
        drive_type: "usb",
        removable: true,
        read_only: false,
        label: null,
        uuid: null,
      },
    ];
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "transfer_preflight") {
        return { can_proceed: true, warnings: [], errors: [], source_drive: null, dest_drive: null };
      }
      return [];
    });

    render(
      <DeviceSidebar
        onNavigate={vi.fn()}
        onDetectDrives={vi.fn(async () => drives)}
        refreshInterval={0}
      />,
    );

    const btn = await screen.findByLabelText("Preflight check for USB Stick");
    fireEvent.click(btn);

    // totalBytes was previously missing — the backend rejected the call
    // and preflight silently returned null.
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith(
        "transfer_preflight",
        expect.objectContaining({
          sourcePath: "/",
          destPath: "/Volumes/USB",
          totalBytes: 600_000,
        }),
      ),
    );
  });
});
