/**
 * Legal hold and eDiscovery export: commands and payloads match the Rust
 * side (`backup_place_legal_hold`, `backup_release_legal_hold`,
 * `ediscovery_export` with an `EDiscoveryRequest`).
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...a: unknown[]) => mockInvoke(...a),
}));

import { BackupLegalHold } from "@/components/backup/backup-legal-hold";
import {
  newBackupJob,
  useBackupStore,
  type BackupJob,
} from "@/stores/backup-store";

const held = (j: BackupJob): BackupJob => ({
  ...j,
  legal_hold: {
    matter: "Case 17",
    reason: "Litigation",
    placed_by: "user",
    placed_at: "2026-10-01T10:00:00Z",
  },
});

beforeEach(() => {
  mockInvoke.mockReset();
  useBackupStore.setState({ jobs: [], error: null });
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

describe("legal hold", () => {
  it("new jobs are not held", () => {
    expect(newBackupJob().legal_hold).toBeNull();
  });

  it("places a hold with matter and reason", async () => {
    const job = { ...newBackupJob(), id: "j1" };
    mockInvoke.mockImplementation(async (cmd: string) =>
      cmd === "backup_place_legal_hold"
        ? held(job)
        : cmd === "backup_list_jobs"
          ? [held(job)]
          : null,
    );
    render(<BackupLegalHold job={job} />);
    expect(screen.getByTestId("legal-hold-place")).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Matter"), {
      target: { value: "Case 17" },
    });
    fireEvent.change(screen.getByLabelText("Reason"), {
      target: { value: "Litigation" },
    });
    fireEvent.click(screen.getByTestId("legal-hold-place"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("backup_place_legal_hold", {
        id: "j1",
        matter: "Case 17",
        reason: "Litigation",
      }),
    );
    await waitFor(() =>
      expect(useBackupStore.getState().jobs[0].legal_hold?.matter).toBe(
        "Case 17",
      ),
    );
  });

  it("shows an active hold and releases it after confirmation", async () => {
    const job = held({ ...newBackupJob(), id: "j1" });
    mockInvoke.mockImplementation(async (cmd: string) =>
      cmd === "backup_release_legal_hold"
        ? { ...job, legal_hold: null }
        : cmd === "backup_list_jobs"
          ? []
          : null,
    );
    const confirm = vi
      .spyOn(window, "confirm")
      .mockReturnValueOnce(false)
      .mockReturnValueOnce(true);
    render(<BackupLegalHold job={job} />);
    expect(screen.getByTestId("legal-hold-active")).toHaveTextContent(
      "Held for Case 17",
    );
    fireEvent.click(screen.getByTestId("legal-hold-release"));
    expect(mockInvoke).not.toHaveBeenCalledWith(
      "backup_release_legal_hold",
      expect.anything(),
    );
    fireEvent.click(screen.getByTestId("legal-hold-release"));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("backup_release_legal_hold", {
        id: "j1",
      }),
    );
    confirm.mockRestore();
  });

  it("surfaces backend refusals", async () => {
    const job = { ...newBackupJob(), id: "j1" };
    mockInvoke.mockRejectedValue({
      message: "Give the matter or case reference",
      advice: "",
    });
    render(<BackupLegalHold job={job} />);
    fireEvent.change(screen.getByLabelText("Matter"), {
      target: { value: "x" },
    });
    fireEvent.click(screen.getByTestId("legal-hold-place"));
    expect(await screen.findByText(/Give the matter/)).toBeInTheDocument();
  });
});

describe("eDiscovery export", () => {
  it("sends the request for this job using the hold's matter and shows the report", async () => {
    const job = held({ ...newBackupJob(), id: "j1" });
    mockInvoke.mockResolvedValue({
      output: "/x/m.zip",
      sha256: "ab".repeat(32),
      bytes: 2048,
      entries: 9,
      ledger_events: 3,
      jobs: ["Docs"],
      restored_files: 1,
      files_truncated: false,
      audit_ok: true,
    });
    render(<BackupLegalHold job={job} />);
    expect(screen.getByTestId("ediscovery-export")).toBeDisabled();
    fireEvent.change(screen.getByTestId("ediscovery-output"), {
      target: { value: " /x/m.zip " },
    });
    fireEvent.change(screen.getByLabelText("Paths of interest"), {
      target: { value: "/data/legal\n\n /data/hr " },
    });
    fireEvent.change(screen.getByLabelText("From"), {
      target: { value: "2026-01-01T00:00" },
    });
    fireEvent.click(screen.getByLabelText(/Include the files/));
    fireEvent.click(screen.getByTestId("ediscovery-export"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledTimes(1));
    const [cmd, args] = mockInvoke.mock.calls[0] as [
      string,
      { request: Record<string, unknown> },
    ];
    expect(cmd).toBe("ediscovery_export");
    expect(args.request).toEqual({
      matter: "Case 17",
      output: "/x/m.zip",
      paths: ["/data/legal", "/data/hr"],
      since: new Date("2026-01-01T00:00").toISOString(),
      until: null,
      job_ids: ["j1"],
      include_files: true,
      max_file_bytes: null,
    });
    expect(await screen.findByTestId("ediscovery-report")).toHaveTextContent(
      "9 entries, 3 file operations, 1 files",
    );
  });

  it("needs a matter when the job is not held", () => {
    render(<BackupLegalHold job={{ ...newBackupJob(), id: "j1" }} />);
    fireEvent.change(screen.getByTestId("ediscovery-output"), {
      target: { value: "/x/m.zip" },
    });
    expect(screen.getByTestId("ediscovery-export")).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Matter"), {
      target: { value: "Case 9" },
    });
    expect(screen.getByTestId("ediscovery-export")).toBeEnabled();
  });
});
