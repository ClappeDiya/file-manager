import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { useTransferPolling } from "@/hooks/use-transfer-polling";
import { TransferWorkspace } from "@/components/transfer-workspace";
import { transferSummary } from "@/lib/transfer-progress";
import {
  useTransferWorkspace,
  type WorkspaceJob,
  type TransferSession,
} from "@/stores/transfer-workspace-store";
import { SimpleSidebar } from "@/components/simple-mode/simple-sidebar";
import { useUIStore } from "@/stores/ui-store";
import { useFocusTrap } from "@/hooks/use-focus-trap";
const invoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invoke(...args),
}));
vi.mock("@/lib/folder-picker", () => ({
  pickSaveFile: vi.fn(async () => "/reports/transfer.json"),
}));
const job = (
  id: string,
  status = "active",
  done = 0,
  total = 100,
): WorkspaceJob => ({
  id,
  source_path: `/source/${id}`,
  dest_path: `/destination/${id}`,
  total_bytes: total,
  transferred_bytes: done,
  status,
  error_message: null,
});
const session = (jobs: WorkspaceJob[]): TransferSession => ({
  id: "session",
  source: "/source",
  sourceFolder: "/source",
  destination: "/destination",
  jobs,
  mode: "copy",
  foldersCreated: 2,
  movedCount: 0,
  createdAt: new Date().toISOString(),
});
beforeEach(() => {
  invoke.mockReset();
  useTransferWorkspace.setState({ sessions: [], selectedId: null });
  useUIStore.setState({ appMode: "simple" });
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});
describe("truthful transfer results", () => {
  it("weights progress by bytes rather than giving small files equal weight", () => {
    expect(
      transferSummary([
        job("small", "completed", 1, 1),
        job("large", "active", 0, 99),
      ]).percent,
    ).toBe(1);
  });
  it("keeps progress separate from completed-file success and checksum evidence", () => {
    const result = transferSummary([
      job("a", "completed", 100),
      job("b", "failed", 50),
      job("c", "cancelled", 0),
      job("d", "active", 50),
    ]);
    expect(result).toMatchObject({
      percent: 50,
      successPercent: 25,
      completed: 1,
      failed: 1,
      cancelled: 1,
      verified: 0,
      settled: false,
    });
  });
  it("never infers verification from a requested flag or a completed state", () => {
    expect(
      transferSummary([
        {
          ...job("a", "completed", 100),
          source_checksum: "one",
          dest_checksum: "two",
        },
      ]).verified,
    ).toBe(0);
    expect(
      transferSummary([
        {
          ...job("a", "completed", 100),
          source_checksum: "one",
          dest_checksum: "one",
        },
      ]).verified,
    ).toBe(1);
  });
  it("does not invent a percentage before the size is known", () => {
    render(
      <TransferWorkspace
        jobs={[job("a", "queued", 0, 0)]}
        error={null}
        onNewTransfer={vi.fn()}
      />,
    );
    expect(screen.getByRole("progressbar")).not.toHaveAttribute(
      "aria-valuenow",
    );
    expect(screen.getByText("Calculating size…")).toBeInTheDocument();
  });
  it("clamps byte progress to the measured total", () => {
    expect(transferSummary([job("a", "active", 500, 100)]).percent).toBe(100);
    expect(transferSummary([job("a", "active", -5, 100)]).percent).toBe(0);
  });
  it("retains mixed failure results and exports the same underlying jobs", async () => {
    const jobs = [
      job("a", "completed", 100),
      { ...job("b", "failed", 0), error_message: "Disk full" },
    ];
    useTransferWorkspace.getState().add(session(jobs));
    render(
      <TransferWorkspace jobs={jobs} error={null} onNewTransfer={vi.fn()} />,
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "Finished with failures",
    );
    expect(screen.getByText("50% file success")).toBeInTheDocument();
    expect(screen.getByRole("progressbar")).toHaveAttribute(
      "aria-valuenow",
      "50",
    );
    fireEvent.click(screen.getByRole("button", { name: "File results" }));
    expect(screen.getByText("Disk full")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Save report" }));
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith(
        "write_text_file_full",
        expect.objectContaining({ path: "/reports/transfer.json" }),
      ),
    );
    const content = JSON.parse(
      invoke.mock.calls.find(([cmd]) => cmd === "write_text_file_full")![1]
        .content,
    );
    expect(content.result.successPercent).toBe(50);
    expect(content.jobs).toEqual(jobs);
    expect(
      screen.getByText("Report saved to /reports/transfer.json"),
    ).toBeInTheDocument();
    act(() =>
      useTransferWorkspace
        .getState()
        .add({ ...session([job("new")]), id: "new-session" }),
    );
    expect(
      screen.queryByText("Report saved to /reports/transfer.json"),
    ).not.toBeInTheDocument();
    expect(screen.queryByRole("list", { name: "Individual file results" })).not.toBeInTheDocument();
  });
  it("shows stale status explicitly instead of claiming a fresh success", () => {
    useTransferWorkspace.getState().add(session([job("a", "completed", 100)]));
    render(
      <TransferWorkspace
        jobs={[]}
        error="Connection lost"
        onNewTransfer={vi.fn()}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("Status unavailable");
    expect(screen.getByRole("alert")).toHaveTextContent("Connection lost");
  });
  it("shows a warning if a completed move kept its source", () => {
    const jobs = [
      {
        ...job("a", "completed", 100),
        error_message: "Copied; original kept: permission denied",
      },
    ];
    render(
      <TransferWorkspace jobs={jobs} error={null} onNewTransfer={vi.fn()} />,
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "Finished with warnings",
    );
  });
  it("marks an unfinished saved transfer unavailable when it disappears from the queue", () => {
    useTransferWorkspace.getState().add(session([job("a", "active", 50)]));
    useTransferWorkspace.getState().updateJobs([]);
    render(
      <TransferWorkspace jobs={[]} error={null} onNewTransfer={vi.fn()} />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("Status unavailable");
    expect(screen.getByRole("alert")).toHaveTextContent(
      "no longer in the current queue",
    );
    expect(
      screen.queryByRole("button", { name: "Pause" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Cancel" }),
    ).not.toBeInTheDocument();
    useTransferWorkspace.getState().updateJobs([job("a", "active", 75)]);
    expect(
      useTransferWorkspace.getState().sessions[0].jobs[0].status_available,
    ).toBe(true);
  });
  it("keeps a mixed-warning transfer in progress until its remaining files settle", () => {
    render(
      <TransferWorkspace
        jobs={[
          { ...job("a", "completed", 100), error_message: "Original kept" },
          job("b", "active", 50),
        ]}
        error={null}
        onNewTransfer={vi.fn()}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "Transfer has warnings",
    );
    expect(screen.queryByText(/file success/)).not.toBeInTheDocument();
  });
  it("retries only failed controllable jobs and surfaces a rejected retry", async () => {
    invoke.mockResolvedValue(false);
    render(
      <TransferWorkspace
        jobs={[job("a", "failed"), job("b", "completed", 100)]}
        error={null}
        onNewTransfer={vi.fn()}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Retry failed" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("retry limit");
    expect(invoke.mock.calls).toEqual([["retry_transfer", { jobId: "a" }]]);
  });
  it("does not hide cancelled items in a 100% success claim", () => {
    render(
      <TransferWorkspace
        jobs={[job("a", "completed", 100), job("b", "cancelled")]}
        error={null}
        onNewTransfer={vi.fn()}
      />,
    );
    expect(screen.getByText("50% file success")).toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent("Transfer cancelled");
  });
  it("polls without overlap and refreshes the destination when a file completes", async () => {
    vi.useFakeTimers();
    try {
      let resolve!: (jobs: WorkspaceJob[]) => void;
      invoke.mockImplementationOnce(
        () =>
          new Promise((r) => {
            resolve = r;
          }),
      );
      const refresh = vi.fn();
      window.addEventListener("ufop:refresh-directory", refresh);
      function Poller() {
        useTransferPolling();
        return null;
      }
      const view = render(<Poller />);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000);
      });
      expect(invoke).toHaveBeenCalledTimes(1);
      await act(async () => {
        resolve([job("a", "completed", 100)]);
      });
      expect(refresh).toHaveBeenCalledOnce();
      invoke.mockResolvedValue([job("a", "completed", 100)]);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(500);
      });
      expect(refresh).toHaveBeenCalledOnce();
      invoke.mockResolvedValue([job("a", "completed", 100), job("cancelled", "cancelled", 30)]);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(500);
      });
      expect(refresh).toHaveBeenCalledTimes(2);
      view.unmount();
      window.removeEventListener("ufop:refresh-directory", refresh);
    } finally {
      vi.useRealTimers();
    }
  });
});
describe("dashboard navigation and focus", () => {
  it("shows grouped action labels in both modes, including backup and restore", () => {
    const view = render(
      <SimpleSidebar activeSection="files" onSectionChange={vi.fn()} />,
    );
    expect(
      screen.getByRole("button", { name: "Backups & Restore" }),
    ).toBeVisible();
    expect(
      screen.getByRole("heading", { name: "File operations" }),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "Files" })).toHaveAttribute(
      "aria-current",
      "page",
    );
    act(() => useUIStore.getState().setAppMode("advanced"));
    view.rerender(
      <SimpleSidebar activeSection="files" onSectionChange={vi.fn()} />,
    );
    expect(
      screen.getByRole("button", { name: "Backups & Restore" }),
    ).toBeVisible();
  });
  it("does not reset input focus when a modal callback changes identity", async () => {
    const close = vi.fn();
    function Dialog({ value }: { value: string }) {
      const ref = useFocusTrap<HTMLDivElement>({
        active: true,
        onEscape: () => close(value),
      });
      return (
        <div ref={ref}>
          <input aria-label="First" />
          <input aria-label="Second" />
        </div>
      );
    }
    const view = render(<Dialog value="before" />);
    await waitFor(() =>
      expect(screen.getByRole("textbox", { name: "First" })).toHaveFocus(),
    );
    screen.getByRole("textbox", { name: "Second" }).focus();
    view.rerender(<Dialog value="after" />);
    await new Promise((r) => requestAnimationFrame(r));
    expect(screen.getByRole("textbox", { name: "Second" })).toHaveFocus();
    fireEvent.keyDown(document, { key: "Escape" });
    expect(close).toHaveBeenCalledWith("after");
  });
});
