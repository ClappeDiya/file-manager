/** WS-I row 74: the in-app log viewer (Advanced). */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { LogViewer } from "@/components/log-viewer";
import { LogViewerDialog } from "@/components/log-viewer-dialog";
import { getDefaultCommands } from "@/components/command-palette";
import { clockTime, localToIso, logQuery, mergeTail, NO_FILTERS, recordText, type LogRecord } from "@/lib/log-viewer";
import { useLogViewer } from "@/stores/log-viewer-store";
import { useUIStore } from "@/stores/ui-store";

const rec = (n: number, extra: Partial<LogRecord> = {}): LogRecord => ({
  id: `2026-10-24@${n * 100}`, ts: `2026-10-24T10:00:${String(n).padStart(2, "0")}.000000Z`, level: "INFO", target: "unified_file_ops_lib::sync_engine", message: `message ${n}`, fields: null, spans: [], ...extra,
});
const page = (records: LogRecord[], extra: Record<string, unknown> = {}) => ({ records, next_cursor: null, sources: ["unified_file_ops_lib::sync_engine", "unified_file_ops_lib::storage"], scan_limited: false, directory: "/home/me/.local/share/ufop/logs", ...extra });

let answer: (q: Record<string, unknown>) => unknown;
const queries = () => mockInvoke.mock.calls.filter(([c]) => c === "query_logs").map(([, a]) => (a as { query: Record<string, unknown> }).query);

beforeEach(() => {
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  // Newest first, as the backend answers.
  answer = () => page([rec(3), rec(2), rec(1)]);
  mockInvoke.mockImplementation(async (cmd: string, args: { query?: Record<string, unknown> }) => {
    if (cmd === "query_logs") return answer(args.query ?? {});
    if (cmd === "reveal_in_os") return null;
    return null;
  });
});

describe("helpers", () => {
  it("builds the query from the filters and leaves empty ones out", () => {
    expect(logQuery(NO_FILTERS, { limit: 500 })).toEqual({ query: { limit: 500 } });
    const q = logQuery({ level: "warn", source: " sync ", text: " disk ", since: "2026-10-24T10:00", until: "" }, { limit: 10 }).query;
    expect(q).toMatchObject({ level: "warn", source: "sync", text: "disk", limit: 10 });
    expect(q.since).toBe(new Date("2026-10-24T10:00").toISOString());
    expect(q.until).toBeUndefined();
    expect(localToIso("nonsense")).toBeNull();
  });

  it("merges a live tail without repeating lines it already shows", () => {
    const shown = [rec(1), rec(2)];
    // The tail asks from the newest line's time, so it gets that line back.
    expect(mergeTail(shown, [rec(4), rec(3), rec(2)]).map((r) => r.message)).toEqual(["message 1", "message 2", "message 3", "message 4"]);
    expect(mergeTail(shown, [rec(2)])).toBe(shown);
    expect(mergeTail(shown, [])).toBe(shown);
  });

  it("formats times in the computer's time zone and records for the clipboard", () => {
    const d = new Date("2026-10-24T10:00:03.000000Z");
    expect(clockTime(rec(3).ts)).toBe(`${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}:03.000`);
    const text = recordText(rec(3, { level: "WARN", spans: ["backup.operation"], fields: { job: "Docs" } }));
    expect(text).toContain(" WARN backup.operation: unified_file_ops_lib::sync_engine: message 3 {\"job\":\"Docs\"}");
  });
});

describe("LogViewer", () => {
  it("shows the newest lines at the bottom and says where the logs are", async () => {
    render(<LogViewer />);
    const list = await screen.findByRole("listbox", { name: "Log lines" });
    await waitFor(() => expect(within(list).getAllByRole("option")).toHaveLength(3));
    expect(within(list).getAllByRole("option").map((o) => /message (\d)/.exec(o.textContent ?? "")?.[1])).toEqual(["1", "2", "3"]);
    expect(queries()[0]).toEqual({ limit: 500 });
    expect(screen.getByText("/home/me/.local/share/ufop/logs")).toBeInTheDocument();
    expect(screen.getByText(/3 lines shown/)).toBeInTheDocument();
  });

  it("filters by level, source and time, and text once typing pauses", async () => {
    render(<LogViewer />);
    await screen.findByRole("listbox", { name: "Log lines" });
    fireEvent.change(screen.getByLabelText("Level"), { target: { value: "warn" } });
    await waitFor(() => expect(queries().at(-1)).toMatchObject({ level: "warn" }));
    fireEvent.change(screen.getByLabelText("Source"), { target: { value: "storage" } });
    await waitFor(() => expect(queries().at(-1)).toMatchObject({ level: "warn", source: "storage" }));
    fireEvent.change(screen.getByLabelText("From time"), { target: { value: "2026-10-24T09:00" } });
    await waitFor(() => expect(typeof queries().at(-1)!.since).toBe("string"));
    const before = queries().length;
    fireEvent.change(screen.getByLabelText("Search text"), { target: { value: "disk" } });
    expect(queries().length).toBe(before);
    await waitFor(() => expect(queries().at(-1)).toMatchObject({ text: "disk" }), { timeout: 2000 });
    expect(screen.getByText(/\(filtered\)/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Clear filters" }));
    await waitFor(() => expect(queries().at(-1)).toEqual({ limit: 500 }));
  });

  it("offers the modules it has seen as sources", async () => {
    render(<LogViewer />);
    await screen.findByRole("listbox", { name: "Log lines" });
    const options = Array.from(document.querySelectorAll("#log-sources option")).map((o) => (o as HTMLOptionElement).value);
    expect(options).toEqual(["unified_file_ops_lib::sync_engine", "unified_file_ops_lib::storage"]);
  });

  it("loads older lines above the ones shown", async () => {
    answer = (q) => (q.cursor ? page([rec(0)], { next_cursor: null }) : page([rec(3), rec(2), rec(1)], { next_cursor: "2026-10-24@100" }));
    render(<LogViewer />);
    fireEvent.click(await screen.findByRole("button", { name: "Load older lines" }));
    await waitFor(() => expect(screen.getByText(/4 lines shown/)).toBeInTheDocument());
    expect(queries().at(-1)).toEqual({ limit: 500, cursor: "2026-10-24@100" });
    const list = screen.getByRole("listbox", { name: "Log lines" });
    expect(within(list).getAllByRole("option")[0].textContent).toContain("message 0");
    expect(screen.queryByRole("button", { name: "Load older lines" })).toBeNull();
  });

  it("follows the log live, adding only new lines", async () => {
    let calls = 0;
    answer = (q) => {
      if (!q.since) return page([rec(2), rec(1)]);
      calls++;
      // From the newest line's time on: that line again, then a new one.
      return page(calls < 2 ? [rec(2)] : [rec(3), rec(2)]);
    };
    render(<LogViewer tailMs={30} />);
    await screen.findByRole("listbox", { name: "Log lines" });
    expect(screen.getByText(/2 lines shown/)).toBeInTheDocument();
    fireEvent.click(screen.getByLabelText("Live tail"));
    await waitFor(() => expect(screen.getByText(/3 lines shown/)).toBeInTheDocument());
    const asked = queries().filter((q) => q.since);
    expect(asked[0].since).toBe(rec(2).ts);
    // Switching it off stops the polling.
    fireEvent.click(screen.getByLabelText("Live tail"));
    const n = queries().length;
    await act(async () => {
      await new Promise((r) => setTimeout(r, 120));
    });
    expect(queries().length).toBe(n);
  });

  it("copies the selected lines", async () => {
    const writeText = vi.fn(async () => undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    render(<LogViewer />);
    const list = await screen.findByRole("listbox", { name: "Log lines" });
    await waitFor(() => expect(within(list).getAllByRole("option")).toHaveLength(3));
    const rows = within(list).getAllByRole("option");
    fireEvent.click(rows[0]);
    fireEvent.click(rows[2], { shiftKey: true });
    expect(screen.getByRole("button", { name: /Copy selected \(3\)/ })).toBeEnabled();
    fireEvent.click(screen.getByRole("button", { name: /Copy selected/ }));
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    const text = (writeText.mock.calls[0] as unknown as [string])[0];
    expect(text.split("\n")).toHaveLength(3);
    expect(text.split("\n")[0]).toContain("message 1");
    expect(text.split("\n")[2]).toContain("message 3");
  });

  it("shows one selected line in full", async () => {
    answer = () => page([rec(2, { message: "first line\nsecond line", fields: { job: "Docs" } }), rec(1)]);
    render(<LogViewer />);
    const list = await screen.findByRole("listbox", { name: "Log lines" });
    await waitFor(() => expect(within(list).getAllByRole("option")).toHaveLength(2));
    fireEvent.click(within(list).getAllByRole("option")[1]);
    expect(screen.getByTestId("log-detail")).toHaveTextContent("second line");
    expect(screen.getByTestId("log-detail")).toHaveTextContent('"job":"Docs"');
  });

  it("opens the log folder", async () => {
    render(<LogViewer />);
    fireEvent.click(await screen.findByRole("button", { name: "Open folder" }));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("reveal_in_os", { path: "/home/me/.local/share/ufop/logs" }));
  });

  it("says when there is nothing, and why a read failed", async () => {
    answer = () => page([]);
    const { unmount } = render(<LogViewer />);
    expect(await screen.findByTestId("log-empty")).toHaveTextContent("There are no log lines yet.");
    unmount();
    answer = () => {
      throw { message: "Could not read the log: denied", advice: "Check the folder." };
    };
    render(<LogViewer />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not read the log: denied");
  });
});

describe("LogViewerDialog", () => {
  it("opens in Advanced mode only", async () => {
    useUIStore.setState({ appMode: "simple" });
    useLogViewer.setState({ open: true });
    const { rerender } = render(<LogViewerDialog />);
    expect(screen.queryByRole("dialog", { name: "Log viewer" })).toBeNull();
    useUIStore.setState({ appMode: "advanced" });
    rerender(<LogViewerDialog />);
    expect(await screen.findByRole("dialog", { name: "Log viewer" })).toBeInTheDocument();
    fireEvent.click(screen.getByLabelText("Close log viewer"));
    expect(useLogViewer.getState().open).toBe(false);
  });
});

describe("command palette", () => {
  it("lists the log viewer only when the caller offers it (Advanced)", () => {
    expect(getDefaultCommands({}).some((c) => c.id === "open-log-viewer")).toBe(false);
    const open = vi.fn();
    const entry = getDefaultCommands({ onOpenLogViewer: open }).find((c) => c.id === "open-log-viewer");
    expect(entry?.label).toMatch(/Log Viewer/);
    entry?.action();
    expect(open).toHaveBeenCalled();
  });
});
