/**
 * Replicas editor and maintenance destination tools: shapes sent to Rust
 * match the serde types (snake_case tags, replica fields, lock modes).
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { BackupReplicas } from "@/components/backup/backup-replicas";
import { BackupMaintenance } from "@/components/backup/backup-maintenance";
import { BackupAdvancedOptions } from "@/components/backup/backup-advanced-options";
import { newBackupJob, type Replica } from "@/stores/backup-store";

beforeEach(() => {
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

describe("replicas", () => {
  it("adds a replica with the engine's defaults and edits its schedule", () => {
    let value: Replica[] = [];
    const onChange = vi.fn((r: Replica[]) => (value = r));
    const { rerender } = render(<BackupReplicas replicas={value} onChange={onChange} connections={[]} library={[]} />);
    fireEvent.click(screen.getByTestId("replica-add"));
    expect(value[0]).toMatchObject({ id: "", enabled: true, schedule: { type: "after_backup" }, delete_extraneous: true, destination: { type: "local" } });
    rerender(<BackupReplicas replicas={value} onChange={onChange} connections={[]} library={[]} />);
    fireEvent.change(screen.getByDisplayValue("After every backup"), { target: { value: "interval" } });
    expect(value[0].schedule).toEqual({ type: "interval", minutes: 60 });
  });

  it("object lock option maps to ObjectLockOptions", () => {
    const onChange = vi.fn();
    render(<BackupAdvancedOptions options={newBackupJob().options} locked={false} onChange={onChange} />);
    fireEvent.click(screen.getByLabelText("Lock new volumes"));
    expect(onChange).toHaveBeenLastCalledWith({ object_lock: { days: 30, mode: "governance" } });
  });
});

describe("maintenance destination tools", () => {
  it("lists destination files and guards backup files", async () => {
    const job = { ...newBackupJob(), id: "j1", replicas: [] };
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "backup_remote_list")
        return [
          { name: "ufop-repo.json", size: 300 },
          { name: "notes.txt", size: 5 },
        ];
      if (cmd === "backup_remote_delete") return null;
      if (cmd === "backup_execute") return { run_id: "r", status: "success", summary: "3 volume(s) locked until x", report: {}, log: [] };
      return null;
    });
    render(<BackupMaintenance job={job} onDeleted={() => {}} />);
    fireEvent.click(screen.getByTestId("remote-list"));
    expect(await screen.findByText("notes.txt")).toBeInTheDocument();
    expect(screen.getAllByText("delete")).toHaveLength(1);
    fireEvent.click(screen.getByText("delete"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_remote_delete", { destination: job.destination, name: "notes.txt", force: false }));
    fireEvent.click(screen.getAllByTestId("backup-op-apply_locks")[0]);
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("backup_execute", { id: "j1", operation: { type: "apply_locks", days: 30, mode: "governance", all: false } }),
    );
  });
});
