/** WS-I row 84: the terminal's command help drawer. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));
vi.mock("@/components/xterm-view", () => ({ XtermView: () => <div data-testid="xterm" /> }));
vi.mock("../components/xterm-view", () => ({ XtermView: () => <div data-testid="xterm" /> }));

import { TerminalHelpDrawer } from "@/components/terminal-help-drawer";
import { TerminalPanel } from "@/components/terminal-panel";
import { entriesOf, insertText, searchEntries, type TerminalHelp } from "@/lib/terminal-help";
import { useTerminalStore } from "@/stores/terminal-store";

const HELP: TerminalHelp = {
  ufop: [
    { path: ["backup"], about: "Versioned, deduplicated, encrypted backups", usage: "backup <COMMAND> [OPTIONS]", has_subcommands: true, options: [] },
    { path: ["backup", "run"], about: "Run a backup job now", usage: "backup run <JOB> [OPTIONS]", has_subcommands: false, options: [{ flags: ["--dry-run"], help: "Show what would happen", takes_value: false }, { flags: ["--limit"], help: "Rate limit", takes_value: true }] },
    { path: ["sync", "list"], about: "List sync pairs", usage: "sync list", has_subcommands: false, options: [] },
  ],
  rclone: [
    { name: "copy", about: "Copy files from source to dest, skipping identical files." },
    { name: "ls", about: "List the objects in the path with size and path." },
  ],
  rclone_version: "rclone v1.65.0",
  rclone_note: null,
};

let help: TerminalHelp;
beforeEach(() => {
  mockInvoke.mockReset();
  localStorage.clear();
  help = HELP;
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  mockInvoke.mockImplementation(async (cmd: string) => (cmd === "terminal_help_commands" ? help : null));
});

describe("help data", () => {
  it("searches commands and descriptions word by word, ignoring case", () => {
    const all = entriesOf(HELP);
    expect(all.map((e) => e.command)).toEqual(["ufop backup", "ufop backup run", "ufop sync list", "rclone copy", "rclone ls"]);
    expect(searchEntries(all, "").length).toBe(5);
    expect(searchEntries(all, "BACKUP run").map((e) => e.command)).toEqual(["ufop backup run"]);
    expect(searchEntries(all, "identical").map((e) => e.command)).toEqual(["rclone copy"]);
    expect(searchEntries(all, "nothing like this")).toEqual([]);
  });

  it("types the command and a space, never Enter", () => {
    expect(insertText(entriesOf(HELP)[1])).toBe("ufop backup run ");
    expect(insertText(entriesOf(HELP)[3])).toBe("rclone copy ");
  });
});

describe("TerminalHelpDrawer", () => {
  it("lists ufop and rclone commands with one-line descriptions", async () => {
    render(<TerminalHelpDrawer onInsert={vi.fn()} canInsert onClose={vi.fn()} />);
    const ufop = await screen.findByRole("region", { name: "ufop commands" });
    expect(within(ufop).getByText("ufop backup run")).toBeInTheDocument();
    expect(within(ufop).getByText("Run a backup job now")).toBeInTheDocument();
    const rclone = screen.getByRole("region", { name: "rclone commands (rclone v1.65.0)" });
    expect(within(rclone).getByText("rclone copy")).toBeInTheDocument();
  });

  it("filters as you type and inserts the clicked command", async () => {
    const onInsert = vi.fn();
    render(<TerminalHelpDrawer onInsert={onInsert} canInsert onClose={vi.fn()} />);
    await screen.findByText("ufop backup run");
    fireEvent.change(screen.getByLabelText("Search commands"), { target: { value: "sync" } });
    expect(screen.queryByText("ufop backup run")).toBeNull();
    fireEvent.click(screen.getByText("ufop sync list"));
    expect(onInsert).toHaveBeenCalledWith("ufop sync list ");
    fireEvent.change(screen.getByLabelText("Search commands"), { target: { value: "zzzz" } });
    expect(screen.getByText(/No command matches/)).toBeInTheDocument();
  });

  it("shows a command's synopsis and options", async () => {
    render(<TerminalHelpDrawer onInsert={vi.fn()} canInsert onClose={vi.fn()} />);
    await screen.findByText("ufop backup run");
    fireEvent.click(screen.getByLabelText("Details of ufop backup run"));
    const details = screen.getByTestId("help-details");
    expect(details).toHaveTextContent("ufop backup run <JOB> [OPTIONS]");
    expect(details).toHaveTextContent("--dry-run");
    expect(details).toHaveTextContent("--limit <value>");
    // rclone's list carries no synopsis, so it has no details to open.
    expect(screen.getByLabelText("Details of rclone copy")).toBeDisabled();
  });

  it("explains a missing rclone instead of listing nothing", async () => {
    help = { ...HELP, rclone: null, rclone_version: null, rclone_note: "rclone was not found. Choose it in Settings → rclone." };
    render(<TerminalHelpDrawer onInsert={vi.fn()} canInsert onClose={vi.fn()} />);
    expect(await screen.findByTestId("rclone-note")).toHaveTextContent("rclone was not found");
    expect(screen.queryByText("rclone copy")).toBeNull();
    expect(screen.getByText("ufop backup run")).toBeInTheDocument();
  });

  it("will not type anywhere without a terminal session", async () => {
    const onInsert = vi.fn();
    render(<TerminalHelpDrawer onInsert={onInsert} canInsert={false} onClose={vi.fn()} />);
    await screen.findByText("ufop backup run");
    const row = screen.getByText("ufop backup run").closest("button")!;
    expect(row).toBeDisabled();
    fireEvent.click(row);
    expect(onInsert).not.toHaveBeenCalled();
  });

  it("shows why the list could not load", async () => {
    mockInvoke.mockImplementation(async () => {
      throw { message: "Could not run rclone.", advice: "Check Settings." };
    });
    render(<TerminalHelpDrawer onInsert={vi.fn()} canInsert onClose={vi.fn()} />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not run rclone.");
  });
});

describe("TerminalPanel help button", () => {
  it("opens the drawer and types the clicked command into the active session", async () => {
    useTerminalStore.setState({
      panelOpen: true,
      sessions: [{ id: "s1", title: "bash", session_type: "local", shell: "bash", cwd: "/", created_at: "", active: true, cols: 80, rows: 24 } as never],
      activeSessionId: "s1",
      layout: null,
    });
    render(<TerminalPanel />);
    expect(screen.queryByTestId("terminal-help")).toBeNull();
    fireEvent.click(screen.getByTestId("terminal-help-btn"));
    fireEvent.click(await screen.findByText("ufop backup run"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("terminal_write", { sessionId: "s1", data: "ufop backup run " }));
    fireEvent.click(screen.getByLabelText("Close command help"));
    expect(screen.queryByTestId("terminal-help")).toBeNull();
  });
});
