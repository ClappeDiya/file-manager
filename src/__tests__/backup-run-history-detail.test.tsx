/** WS-I row 73: backup run history shows what started a run, filters by date, shows per-destination results and opens the folders. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { BackupRunHistory, replicaResults } from "@/components/backup/backup-run-history";
import { ReplicaHistory } from "@/components/backup/backup-replicas";
import { useBackupStore } from "@/stores/backup-store";
import { useConnectionBrowse } from "@/stores/connection-browse-store";
import { destinationTarget } from "@/lib/open-location";

const replicaRuns = [
  { id: "usb", name: "USB", status: "success", started_at: "2026-10-01T10:00:00Z", finished_at: "2026-10-01T10:00:30Z", files: 4, bytes: 1094, error: null },
  { id: "nas", name: "NAS", status: "error", started_at: "2026-10-01T10:00:31Z", finished_at: "2026-10-01T10:00:32Z", files: 0, bytes: 0, error: "host unreachable" },
];
const run = (id: string, trigger: string | undefined, report: Record<string, unknown> = {}) => ({
  id, job_id: "j1", operation: "backup", status: "success", started_at: "2026-10-01T10:00:00Z", finished_at: "2026-10-01T10:02:05Z", summary: `run ${id}`, report, log: [], ...(trigger ? { trigger } : {}),
});
const job = (destination: unknown) => ({ id: "j1", name: "Docs", sources: ["/home/me/Docs", "/home/me/Pictures"], destination }) as never;

beforeEach(() => {
  localStorage.clear();
  mockInvoke.mockReset();
  Object.defineProperty(window, "__TAURI_INTERNALS__", { value: {}, configurable: true });
  useConnectionBrowse.setState({ request: null });
  useBackupStore.setState({ jobs: [job({ type: "local", path: "/mnt/backup", alternate_paths: [], marker_file: null })] });
  mockInvoke.mockImplementation(async (cmd: string) => {
    if (cmd === "backup_run_history") {
      return { items: [run("a", "scheduled", { replication: [{ replica_runs: replicaRuns }] }), run("b", "api"), run("c", undefined)], total: 3, offset: 0, limit: 50 };
    }
    if (cmd === "backup_list_destinations") return [];
    return null;
  });
});

const calls = (cmd: string) => mockInvoke.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a as Record<string, unknown>);

describe("backup run history detail", () => {
  it("shows a Started by column, Manual for runs stored before triggers existed", async () => {
    render(<BackupRunHistory />);
    await screen.findByText("run a");
    expect(within(screen.getByTestId("run-row-a")).getByText("Scheduled")).toBeInTheDocument();
    expect(within(screen.getByTestId("run-row-b")).getByText("API")).toBeInTheDocument();
    expect(within(screen.getByTestId("run-row-c")).getByText("Manual")).toBeInTheDocument();
  });

  it("passes the date range to the backend and leaves all-time calls without dates", async () => {
    render(<BackupRunHistory />);
    await screen.findByText("run a");
    expect(calls("backup_run_history")[0]).not.toHaveProperty("dateFrom");
    fireEvent.click(screen.getByRole("button", { name: "Yesterday" }));
    await waitFor(() => expect(calls("backup_run_history").some((a) => typeof a.dateFrom === "string" && typeof a.dateTo === "string")).toBe(true));
    const last = calls("backup_run_history").at(-1)!;
    expect(Date.parse(String(last.dateTo))).toBeLessThan(Date.now());
    expect(Date.parse(String(last.dateTo)) - Date.parse(String(last.dateFrom))).toBeGreaterThan(22 * 3600_000);
  });

  it("shows each destination's result for a run that replicated, with the failure's reason", async () => {
    render(<BackupRunHistory />);
    fireEvent.click(await screen.findByText("run a"));
    const table = await screen.findByTestId("replica-results");
    expect(within(table).getByText("USB")).toBeInTheDocument();
    expect(within(table).getByText(/host unreachable/)).toBeInTheDocument();
    expect(screen.getByTestId("run-details")).toHaveTextContent("started by scheduled");
  });

  it("reveals the job's sources and destination folders", async () => {
    render(<BackupRunHistory />);
    fireEvent.click(await screen.findByText("run a"));
    fireEvent.click(await screen.findByRole("button", { name: "Open source: Docs" }));
    await waitFor(() => expect(calls("reveal_in_os")).toEqual([{ path: "/home/me/Docs" }]));
    fireEvent.click(screen.getByRole("button", { name: "Open destination" }));
    await waitFor(() => expect(calls("reveal_in_os")[1]).toEqual({ path: "/mnt/backup" }));
  });

  it("opens a connection destination in the connection browser", async () => {
    useBackupStore.setState({ jobs: [job({ type: "connection", connection_id: "c9", remote_path: "/backups" })] });
    render(<BackupRunHistory />);
    fireEvent.click(await screen.findByText("run a"));
    fireEvent.click(await screen.findByRole("button", { name: "Open destination" }));
    expect(useConnectionBrowse.getState().request).toEqual({ connectionId: "c9", path: "/backups" });
    expect(calls("reveal_in_os")).toEqual([]);
  });

  it("says why a cloud store cannot be opened instead of pretending", async () => {
    useBackupStore.setState({ jobs: [job({ type: "cloud", config: { provider: "s3" }, credential_id: "x" })] });
    render(<BackupRunHistory />);
    fireEvent.click(await screen.findByText("run a"));
    expect(await screen.findByRole("button", { name: "Open destination" })).toBeDisabled();
    expect(screen.getByText(/no saved connection to browse/)).toBeInTheDocument();
  });
});

describe("destinationTarget", () => {
  it("follows a library entry to what it names, with its sub-folder", () => {
    const library = [
      { id: "l1", name: "NAS", destination: { type: "local", path: "/mnt/nas/", alternate_paths: [], marker_file: null }, created_at: "" },
      { id: "l2", name: "SFTP", destination: { type: "connection", connection_id: "c1", remote_path: "/data" }, created_at: "" },
    ] as never;
    expect(destinationTarget({ type: "library", destination_id: "l1", remote_path: "docs" }, library)).toEqual({ kind: "local", path: "/mnt/nas/docs" });
    expect(destinationTarget({ type: "library", destination_id: "l2", remote_path: "docs" }, library)).toEqual({ kind: "connection", connectionId: "c1", path: "/data/docs" });
    expect(destinationTarget({ type: "library", destination_id: "gone", remote_path: "" }, library).kind).toBe("none");
  });
});

describe("replicaResults", () => {
  it("reads a replication's own results and those of the replications a backup ran", () => {
    expect(replicaResults({ replica_runs: [replicaRuns[0]] }).map((r) => r.name)).toEqual(["USB"]);
    expect(replicaResults({ replication: [{ replica_runs: replicaRuns }] }).map((r) => r.name)).toEqual(["USB", "NAS"]);
    expect(replicaResults({})).toEqual([]);
  });
});

describe("ReplicaHistory", () => {
  it("lists every copy to a replica, not only the last one", async () => {
    mockInvoke.mockImplementation(async () => [
      { id: "1", run_id: "r1", job_id: "j1", replica_id: "usb", replica_name: "USB", status: "success", started_at: "2026-10-02T10:00:00Z", finished_at: "2026-10-02T10:00:10Z", files: 0, bytes: 0, error: null, trigger: "api" },
      { id: "2", run_id: "r0", job_id: "j1", replica_id: "usb", replica_name: "USB", status: "error", started_at: "2026-10-01T10:00:00Z", finished_at: "2026-10-01T10:00:02Z", files: 0, bytes: 0, error: "disk full", trigger: "manual" },
    ]);
    render(<ReplicaHistory jobId="j1" replicaId="usb" />);
    const table = await screen.findByTestId("replica-history");
    expect(within(table).getAllByRole("row")).toHaveLength(3);
    expect(within(table).getByText(/disk full/)).toBeInTheDocument();
    expect(within(table).getByText("API")).toBeInTheDocument();
    expect(calls("backup_replica_history")[0]).toEqual({ jobId: "j1", replicaId: "usb", limit: 30 });
  });

  it("says so when nothing was copied yet", async () => {
    mockInvoke.mockImplementation(async () => []);
    render(<ReplicaHistory jobId="j1" replicaId="usb" />);
    expect(await screen.findByText("No copies to this destination yet.")).toBeInTheDocument();
  });
});
