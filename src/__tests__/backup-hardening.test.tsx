/**
 * K87: the job editor's ransomware-protection section: a "Protection: n/m"
 * badge in Standard mode, the checklist with guide links and "Run checks"
 * (confirmed first, never automatic) in Advanced mode.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...a: unknown[]) => mockInvoke(...a),
}));

import { BackupHardening } from "@/components/backup/backup-hardening";
import { DOCS_BASE, protectionTone } from "@/lib/hardening";
import { useUIStore } from "@/stores/ui-store";
import type { BackupJob, HardeningReport } from "@/stores/backup-store";

const report = (credentials: "unknown" | "fail"): HardeningReport => ({
  job_id: "j1",
  passed: 4,
  total: 7,
  probe:
    credentials === "fail"
      ? {
          ran_at: "2026-10-23T10:00:00Z",
          can_write: true,
          can_delete: true,
          gone_after_delete: true,
          leftover: null,
          detail: "deleted",
        }
      : null,
  checks: [
    {
      id: "encryption",
      title: "Backups are encrypted",
      status: "pass",
      detail: "Yes.",
      advice: "",
      doc: "docs/backup-ransomware-hardening.md#encrypt-everything",
    },
    {
      id: "object_lock",
      title: "New volumes are locked (immutable)",
      status: "warn",
      detail: "No lock.",
      advice: "Turn on object lock.",
      doc: "docs/backup-ransomware-hardening.md#object-lock-and-immutability",
    },
    {
      id: "credentials",
      title: "The backup credentials cannot delete",
      status: credentials,
      detail: "…",
      advice: "Run the checks.",
      doc: "docs/backup-ransomware-hardening.md#least-privilege-credentials",
    },
  ],
});

const job = {
  id: "j1",
  name: "Docs",
  read_only: false,
  updated_at: "t",
} as unknown as BackupJob;

beforeEach(() => {
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  mockInvoke.mockImplementation(
    async (cmd: string, args: { probe?: boolean }) => {
      if (cmd === "backup_hardening_report")
        return report(args.probe ? "fail" : "unknown");
      return null;
    },
  );
});

describe("ransomware protection", () => {
  it("shows only the badge in Standard mode", async () => {
    useUIStore.setState({ appMode: "simple", backupAdvanced: false });
    render(<BackupHardening job={job} />);
    expect((await screen.findByTestId("protection-badge")).textContent).toBe(
      "Protection: 4/7",
    );
    expect(screen.queryByTestId("hardening-checks")).toBeNull();
    expect(screen.queryByRole("button", { name: "Run checks" })).toBeNull();
    expect(mockInvoke).toHaveBeenCalledWith("backup_hardening_report", {
      jobId: "j1",
      probe: false,
    });
  });

  it("lists checks with guide links and probes only after confirmation", async () => {
    useUIStore.setState({ appMode: "advanced", backupAdvanced: true });
    render(<BackupHardening job={job} />);
    const list = await screen.findByTestId("hardening-checks");
    expect(within(list).getByText("Backups are encrypted")).toBeTruthy();
    const links = within(list).getAllByRole("link", { name: "Guide" });
    expect(links[0].getAttribute("href")).toBe(
      DOCS_BASE +
        "docs/backup-ransomware-hardening.md#object-lock-and-immutability",
    );
    expect(mockInvoke.mock.calls.every((c) => c[1].probe === false)).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Run checks" }));
    expect(mockInvoke.mock.calls.every((c) => c[1].probe === false)).toBe(true);
    fireEvent.click(screen.getByRole("button", { name: "Run now" }));
    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith("backup_hardening_report", {
        jobId: "j1",
        probe: true,
      }),
    );
    await waitFor(() => expect(screen.getByText(/Last checked/)).toBeTruthy());
    expect(
      within(screen.getByTestId("hardening-checks")).getAllByLabelText("fail")
        .length,
    ).toBe(1);
  });

  it("does not probe read-only jobs and renders nothing for unsaved jobs", async () => {
    useUIStore.setState({ appMode: "advanced", backupAdvanced: true });
    const { unmount } = render(
      <BackupHardening job={{ ...job, read_only: true }} />,
    );
    expect(
      (
        (await screen.findByRole("button", {
          name: "Run checks",
        })) as HTMLButtonElement
      ).disabled,
    ).toBe(true);
    unmount();
    const { container } = render(<BackupHardening job={{ ...job, id: "" }} />);
    expect(container.textContent).toBe("");
  });

  it("colours the badge by the worst result", () => {
    expect(protectionTone(report("fail"))).toContain("red");
    expect(protectionTone(report("unknown"))).toContain("amber");
    expect(
      protectionTone({ ...report("unknown"), passed: 7, total: 7, checks: [] }),
    ).toContain("green");
  });
});
