import { describe, expect, it, vi, beforeEach, beforeAll } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { ConnectionPanel } from "@/components/connection-panel";
import { PromptDialogHost } from "@/components/prompt-dialog-host";
import { pickFiles, pickFolder } from "@/lib/folder-picker";

const mockInvoke = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@/lib/folder-picker", () => ({
  pickFiles: vi.fn(),
  pickFolder: vi.fn(),
}));

const CONN = {
  id: "conn-1",
  name: "Test SFTP",
  protocol: "sftp",
  host: "example.test",
  port: 22,
  username: "u",
  credential_ref: null,
  remote_path: "/",
  created_at: "",
  last_used: null,
  group_id: null,
  bandwidth_limit_bps: 0,
  verify_checksums: false,
};

const LISTING = [
  { name: "subdir", path: "/subdir", is_dir: true, size: 0, modified: null },
  { name: "report.txt", path: "/report.txt", is_dir: false, size: 512, modified: null },
];

function backendFor(cmd: string): unknown {
  switch (cmd) {
    case "list_connections":
      return [CONN];
    case "list_connection_groups":
      return [];
    case "connector_list_protocols":
      return ["sftp"];
    case "get_connection":
      return CONN;
    case "connector_connect":
      return undefined;
    case "connector_list_remote":
      return LISTING;
    case "connector_is_connected":
      return true;
    case "connector_upload":
      return 100;
    case "connector_download":
      return 512;
    case "connector_make_directory":
    case "connector_remove":
      return undefined;
    default:
      return undefined;
  }
}

const calls = (cmd: string) =>
  mockInvoke.mock.calls.filter((c) => c[0] === cmd).map((c) => c[1]);

async function connectAndBrowse() {
  // Clicking the saved connection's row connects and lists the remote root.
  const row = await screen.findByText("Test SFTP");
  fireEvent.click(row);
  await screen.findByText("report.txt");
}

beforeAll(() => {
  // tauriInvoke only calls through when it thinks it's inside Tauri.
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

beforeEach(() => {
  mockInvoke.mockReset();
  mockInvoke.mockImplementation((cmd: string) => Promise.resolve(backendFor(cmd)));
  vi.mocked(pickFiles).mockReset();
  vi.mocked(pickFolder).mockReset();
});

describe("ConnectionPanel remote browser", () => {
  it("connects and lists remote files", async () => {
    render(
      <>
        <ConnectionPanel />
        <PromptDialogHost />
      </>,
    );
    await connectAndBrowse();
    expect(calls("connector_connect")).toEqual([
      expect.objectContaining({ protocol: "sftp", connectionId: "conn-1" }),
    ]);
    expect(calls("connector_list_remote")).toEqual([
      expect.objectContaining({ protocol: "sftp", path: "/", connectionId: "conn-1" }),
    ]);
  });

  it("uploads picked files into the current remote directory", async () => {
    vi.mocked(pickFiles).mockResolvedValue(["/local/a.bin", "/local/b.bin"]);
    render(
      <>
        <ConnectionPanel />
        <PromptDialogHost />
      </>,
    );
    await connectAndBrowse();

    fireEvent.click(screen.getByTestId("remote-upload"));
    await screen.findByText(/Uploaded 2 files/);

    expect(calls("connector_upload")).toEqual([
      expect.objectContaining({ localPath: "/local/a.bin", remotePath: "/a.bin", connectionId: "conn-1" }),
      expect.objectContaining({ localPath: "/local/b.bin", remotePath: "/b.bin", connectionId: "conn-1" }),
    ]);
    // The remote listing was refreshed afterwards.
    expect(calls("connector_list_remote").length).toBeGreaterThanOrEqual(2);
  });

  it("downloads a remote file into the picked folder", async () => {
    vi.mocked(pickFolder).mockResolvedValue("/downloads");
    render(
      <>
        <ConnectionPanel />
        <PromptDialogHost />
      </>,
    );
    await connectAndBrowse();

    fireEvent.click(screen.getByTestId("remote-download-report.txt"));
    await screen.findByText(/Downloaded report\.txt/);

    expect(calls("connector_download")).toEqual([
      expect.objectContaining({
        remotePath: "/report.txt",
        localPath: "/downloads/report.txt",
        connectionId: "conn-1",
      }),
    ]);
  });

  it("creates a remote folder through the prompt dialog", async () => {
    render(
      <>
        <ConnectionPanel />
        <PromptDialogHost />
      </>,
    );
    await connectAndBrowse();

    fireEvent.click(screen.getByTestId("remote-new-folder"));
    const input = await screen.findByLabelText("New remote folder name:");
    fireEvent.change(input, { target: { value: "newfolder" } });
    fireEvent.click(screen.getByText("OK"));

    await waitFor(() =>
      expect(calls("connector_make_directory")).toEqual([
        expect.objectContaining({ remotePath: "/newfolder", connectionId: "conn-1" }),
      ]),
    );
  });

  it("deletes a remote file only after confirm", async () => {
    render(
      <>
        <ConnectionPanel />
        <PromptDialogHost />
      </>,
    );
    await connectAndBrowse();

    fireEvent.click(screen.getByTestId("remote-delete-report.txt"));
    // The in-app confirm dialog appears; Cancel first — nothing is deleted.
    let dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByText("Cancel"));
    expect(calls("connector_remove")).toEqual([]);

    fireEvent.click(screen.getByTestId("remote-delete-report.txt"));
    dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByText("Delete"));

    await waitFor(() =>
      expect(calls("connector_remove")).toEqual([
        expect.objectContaining({ remotePath: "/report.txt", isDir: false, connectionId: "conn-1" }),
      ]),
    );
  });

  // The connector writes a native Google file exactly where it is told,
  // so the panel names the export with the connection's format.
  it("downloads a native Google file with its export format's extension", async () => {
    const gconn = { ...CONN, id: "g-1", name: "My Drive", protocol: "google_drive", host: "", extras: { google_drive: { root: "my_drive", shared_drive_id: null, shared_drive_name: null, export_formats: { document: "pdf", spreadsheet: null, presentation: null, drawing: null } } } };
    const listing = [
      { name: "Plan", path: "/f1", is_dir: false, size: 0, modified: null, extension: "gdoc" },
      { name: "Budget", path: "/f2", is_dir: false, size: 0, modified: null, extension: "gsheet" },
    ];
    mockInvoke.mockImplementation((cmd: string) => {
      if (cmd === "list_connections") return Promise.resolve([gconn]);
      if (cmd === "get_connection") return Promise.resolve(gconn);
      if (cmd === "connector_list_protocols") return Promise.resolve(["google_drive"]);
      if (cmd === "connector_list_remote") return Promise.resolve(listing);
      return Promise.resolve(backendFor(cmd));
    });
    vi.mocked(pickFolder).mockResolvedValue("/downloads");
    render(
      <>
        <ConnectionPanel />
        <PromptDialogHost />
      </>,
    );
    fireEvent.click(await screen.findByText("My Drive"));
    await screen.findByText("Plan");
    fireEvent.click(screen.getByTestId("remote-download-Plan"));
    await screen.findByText(/Downloaded Plan\.pdf/);
    fireEvent.click(screen.getByTestId("remote-download-Budget"));
    await screen.findByText(/Downloaded Budget\.xlsx/);
    expect(calls("connector_download")).toEqual([
      expect.objectContaining({ protocol: "google_drive", remotePath: "/f1", localPath: "/downloads/Plan.pdf", connectionId: "g-1" }),
      expect.objectContaining({ protocol: "google_drive", remotePath: "/f2", localPath: "/downloads/Budget.xlsx", connectionId: "g-1" }),
    ]);
  });
});
