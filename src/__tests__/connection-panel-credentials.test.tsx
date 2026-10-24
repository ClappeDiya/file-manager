/**
 * Connections panel: starred connections and password reveal (item 23),
 * profile fields that used to be dropped (item 27: SFTP key passphrase,
 * WebDAV auth/vendor, SMB domain, B2 key), the credentials-rejected flag
 * with Retry (item 87), cloud sign-in entry points and scopes (items 24/25),
 * quota and folder size (item 19).
 */
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { ConnectionPanel } from "@/components/connection-panel";
import { useUIStore } from "@/stores/ui-store";

const base = {
  port: null,
  credential_ref: null,
  remote_path: "/",
  created_at: "2024-01-01T00:00:00Z",
  last_used: null,
  group_id: null,
  bandwidth_limit_bps: 0,
  verify_checksums: false,
};
const ALPHA = { ...base, id: "a", name: "Alpha", protocol: "sftp", host: "alpha.example", username: "u" };
const ZULU = { ...base, id: "z", name: "Zulu", protocol: "webdav", host: "https://dav.example", username: "u" };
const DRIVE = { ...base, id: "g", name: "My Drive", protocol: "google_drive", host: "", username: null, extras: { assume_role: null, rclone_remote: null, preset: null, swift: null, oauth: null, google_drive: null } };

let statuses: Record<string, unknown>[] = [];
let connections: Record<string, unknown>[] = [];
const calls = (cmd: string) => mockInvoke.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a);

function backend(extra: Record<string, (a: Record<string, unknown>) => unknown> = {}) {
  mockInvoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
    if (extra[cmd]) return extra[cmd](args);
    switch (cmd) {
      case "list_connections":
        return connections;
      case "list_connection_status":
        return statuses;
      case "list_connection_groups":
      case "connection_presets":
      case "list_aws_profiles":
        return [];
      case "connector_list_protocols":
        return ["sftp", "webdav", "smb", "b2", "google_drive", "dropbox", "onedrive"];
      case "get_connection":
        return connections.find((c) => c.id === args.connectionId);
      case "connector_list_remote":
        return [{ name: "Photos", path: "/Photos", is_dir: true, size: 0, modified: null }];
      case "save_connection":
        return { ...base, id: "new-id", name: args.name, protocol: args.protocol, host: args.host, username: args.username, extras: args.extras };
      case "oauth_provider_info":
        return { provider: "google_drive", label: "Google Drive", has_builtin_client: true, uses_client_secret: true, redirect_uri: "http://localhost:9876/callback", app_console: "x" };
      default:
        return undefined;
    }
  });
}

async function openNewForm() {
  render(<ConnectionPanel />);
  fireEvent.click(await screen.findByText("Add your first connection"));
  return document.querySelector("form")!;
}

async function chooseProtocol(form: HTMLFormElement, protocol: string) {
  const select = form.querySelector("select")!;
  await waitFor(() => expect([...select.options].some((o) => o.value === protocol)).toBe(true));
  fireEvent.change(select, { target: { value: protocol } });
}

async function savedArgs() {
  await waitFor(() => expect(calls("save_connection").length).toBeGreaterThan(0));
  return calls("save_connection").at(-1)!;
}

beforeAll(() => {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

beforeEach(() => {
  useUIStore.setState({ appMode: "simple", activeGuidedFlow: null });
  statuses = [];
  connections = [];
  mockInvoke.mockReset();
  backend();
});

describe("starred connections and password reveal", () => {
  it("lists starred connections first and toggles the star", async () => {
    connections = [ALPHA, ZULU];
    statuses = [{ id: "z", starred: true, credentials_rejected_at: null, credentials_rejected_reason: null }];
    render(<ConnectionPanel />);
    await screen.findByText("Zulu");
    const names = () => [...document.querySelectorAll("button > div > span.font-medium")].map((n) => n.textContent);
    expect(names()).toEqual(["Zulu", "Alpha"]);
    fireEvent.click(screen.getByRole("button", { name: "Star Alpha" }));
    await waitFor(() => expect(calls("set_connection_starred")).toEqual([{ connectionId: "a", starred: true }]));
    expect(screen.getByRole("button", { name: "Unstar Zulu" })).toHaveAttribute("aria-pressed", "true");
  });

  it("shows and hides the password", async () => {
    await openNewForm();
    const pw = screen.getByLabelText("Password") as HTMLInputElement;
    expect(pw.type).toBe("password");
    fireEvent.click(screen.getAllByRole("button", { name: "Show password" })[0]);
    expect(pw.type).toBe("text");
    fireEvent.click(screen.getAllByRole("button", { name: "Hide password" })[0]);
    expect(pw.type).toBe("password");
  });
});

describe("profile fields reach the backend", () => {
  it("saves the SFTP key passphrase as a secret, not in the profile", async () => {
    const form = await openNewForm();
    fireEvent.change(form.querySelectorAll("input")[0], { target: { value: "Key box" } });
    fireEvent.change(screen.getByLabelText("Password"), { target: { value: "/home/u/.ssh/id_ed25519" } });
    fireEvent.change(screen.getByLabelText(/Key passphrase/), { target: { value: "correct horse" } });
    fireEvent.submit(form);
    const args = await savedArgs();
    expect([args.keyPassphrase, args.password]).toEqual(["correct horse", "/home/u/.ssh/id_ed25519"]);
    expect(JSON.stringify(args.extras)).not.toContain("correct horse");
  });

  it("saves the WebDAV bearer mode and vendor", async () => {
    const form = await openNewForm();
    await chooseProtocol(form, "webdav");
    fireEvent.change(screen.getByLabelText("Sign in with"), { target: { value: "bearer" } });
    fireEvent.change(screen.getByLabelText("Server"), { target: { value: "nextcloud" } });
    expect(screen.queryByLabelText("Username")).toBeNull();
    fireEvent.change(screen.getByLabelText("Bearer token"), { target: { value: "tok" } });
    fireEvent.submit(form);
    const args = await savedArgs();
    expect([args.extras.webdav, args.password]).toEqual([{ auth: "bearer", vendor: "nextcloud" }, "tok"]);
  });

  it("saves the SMB domain", async () => {
    const form = await openNewForm();
    await chooseProtocol(form, "smb");
    fireEvent.change(screen.getByLabelText(/Domain/), { target: { value: " CORP " } });
    fireEvent.submit(form);
    expect((await savedArgs()).extras.smb).toEqual({ domain: "CORP" });
  });

  it("asks B2 for the application key ID and key", async () => {
    const form = await openNewForm();
    await chooseProtocol(form, "b2");
    expect(screen.queryByText("Host")).toBeNull();
    fireEvent.change(screen.getByLabelText("Application key ID"), { target: { value: "0012ab" } });
    fireEvent.change(screen.getByLabelText("Application key"), { target: { value: "K001xyz" } });
    fireEvent.submit(form);
    const args = await savedArgs();
    expect([args.protocol, args.username, args.password]).toEqual(["b2", "0012ab", "K001xyz"]);
  });
});

describe("rejected credentials", () => {
  it("shows the flag and Retry clears it before connecting", async () => {
    connections = [ALPHA];
    statuses = [{ id: "a", starred: false, credentials_rejected_at: "2026-10-01T10:00:00Z", credentials_rejected_reason: "Sign-in rejected: HTTP 401" }];
    render(<ConnectionPanel />);
    expect(await screen.findByTestId("rejected-a")).toHaveTextContent("Credentials rejected");
    fireEvent.click(screen.getByTitle("Retry"));
    await waitFor(() => expect(calls("connector_connect").length).toBe(1));
    const order = mockInvoke.mock.calls.map(([c]) => c);
    expect(order.indexOf("clear_connection_credentials_rejected")).toBeLessThan(order.indexOf("connector_connect"));
    expect(calls("clear_connection_credentials_rejected")).toEqual([{ connectionId: "a" }]);
  });
});

describe("cloud connections", () => {
  it("signs a new Google Drive connection in right after saving it", async () => {
    const form = await openNewForm();
    await chooseProtocol(form, "google_drive");
    expect(screen.queryByLabelText("Password")).toBeNull();
    expect(screen.getByTestId("gdrive-scope")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText("Open"), { target: { value: "shared_with_me" } });
    fireEvent.change(form.querySelectorAll("input")[0], { target: { value: "Work Drive" } });
    fireEvent.submit(form);
    const args = await savedArgs();
    expect(args.extras.google_drive.root).toBe("shared_with_me");
    const dialog = await screen.findByRole("dialog", { name: /Sign in: Work Drive/ });
    expect(await within(dialog).findByRole("button", { name: "Sign in with Google Drive" })).toBeInTheDocument();
  });

  it("offers the custom OAuth client and export formats only in Advanced view", async () => {
    const form = await openNewForm();
    await chooseProtocol(form, "google_drive");
    expect(screen.queryByLabelText(/OAuth client ID/)).toBeNull();
    expect(screen.queryByText("Google Docs export formats")).toBeNull();
    useUIStore.setState({ appMode: "advanced" });
    fireEvent.change(await screen.findByLabelText(/OAuth client ID/), { target: { value: " cid " } });
    fireEvent.click(screen.getByText("Google Docs export formats"));
    fireEvent.change(screen.getByLabelText("Docs"), { target: { value: "pdf" } });
    fireEvent.submit(form);
    const args = await savedArgs();
    expect(args.extras.oauth.client_id).toBe("cid");
    expect(args.extras.google_drive.export_formats.document).toBe("pdf");
  });

  it("opens the cloud wizards from + Cloud", async () => {
    render(<ConnectionPanel />);
    fireEvent.click(await screen.findByRole("button", { name: "+ Cloud" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Dropbox" }));
    expect(useUIStore.getState().activeGuidedFlow).toBe("connect-dropbox");
  });

  it("Sign in on a saved cloud connection opens the sign-in dialog", async () => {
    connections = [DRIVE];
    render(<ConnectionPanel />);
    fireEvent.click(await screen.findByTitle("Sign in"));
    expect(await screen.findByRole("dialog", { name: "Sign in: My Drive" })).toBeInTheDocument();
  });
});

describe("quota and folder size", () => {
  it("shows the account's storage and calculates a folder's size from its context menu", async () => {
    connections = [DRIVE];
    let resolveSize: (v: unknown) => void = () => {};
    backend({
      connector_quota: () => ({ total: 15e9, used: 4e9, free: 11e9, trashed: null }),
      connector_folder_size: () => new Promise((r) => (resolveSize = r)),
      connector_folder_size_cancel: () => true,
    });
    render(<ConnectionPanel />);
    fireEvent.click(await screen.findByText("My Drive"));
    expect(await screen.findByTestId("remote-quota")).toHaveTextContent(/used/);
    expect(calls("connector_quota")).toEqual([{ protocol: "google_drive", connectionId: "g" }]);
    fireEvent.contextMenu(await screen.findByText("Photos"));
    fireEvent.click(screen.getByRole("menuitem", { name: "Calculate size" }));
    expect(await screen.findByTestId("size-Photos")).toHaveTextContent("Calculating");
    const { jobId } = calls("connector_folder_size")[0] as { jobId: string };
    expect(calls("connector_folder_size")[0]).toEqual({ protocol: "google_drive", path: "/Photos", jobId });
    fireEvent.click(screen.getByRole("button", { name: "Stop calculating the size of Photos" }));
    await waitFor(() => expect(calls("connector_folder_size_cancel")).toEqual([{ jobId }]));
    resolveSize({ bytes: 2048, files: 3, folders: 1, complete: false });
    expect(await screen.findByTestId("size-Photos")).toHaveTextContent(/in 3 files \(stopped\)/);
  });
});
