import { describe, expect, it, vi, beforeEach, beforeAll } from "vitest";
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { ConnectionPanel } from "@/components/connection-panel";
import { useUIStore } from "@/stores/ui-store";
import { useRcloneUi } from "@/stores/rclone-ui-store";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => {}) }));

const CRYPT_CONN = {
  id: "conn-sec",
  name: "Secret vault",
  protocol: "rclone",
  host: "sec:",
  port: null,
  username: null,
  credential_ref: null,
  remote_path: "/",
  created_at: "",
  last_used: null,
  group_id: null,
  bandwidth_limit_bps: 0,
  verify_checksums: false,
  extras: { rclone_remote: "sec:", assume_role: null, preset: null, swift: null },
};

function backend(cmd: string, args?: Record<string, unknown>): unknown {
  switch (cmd) {
    case "list_connections":
      return [CRYPT_CONN];
    case "list_connection_groups":
      return [];
    case "connector_list_protocols":
      return ["sftp", "rclone"];
    case "get_connection":
      return CRYPT_CONN;
    case "connector_list_remote":
      return args?.connectionId ? [{ name: "plain.txt", path: "/plain.txt", is_dir: false, size: 3, modified: null }] : [{ name: "v0lg3ncrypt3d", path: "/v0lg3ncrypt3d", is_dir: true, size: 0, modified: null }];
    case "rclone_crypt_info":
      return { remote: "sec", is_crypt: true, underlying: "/data/vault", underlying_remote: ":local:/data/vault", filename_encryption: "standard", directory_name_encryption: true };
    case "rclone_list_remotes":
      return [{ name: "al:", backend: "alias" }];
    case "rclone_providers":
      return [{ name: "alias", description: "Alias", prefix: "alias", hidden: false, options: [] }];
    case "connection_presets":
      return [];
    default:
      return undefined;
  }
}

const calls = (cmd: string) => mockInvoke.mock.calls.filter((c) => c[0] === cmd).map((c) => c[1]);

beforeAll(() => {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

beforeEach(() => {
  mockInvoke.mockReset();
  mockInvoke.mockImplementation((cmd: string, args?: Record<string, unknown>) => Promise.resolve(backend(cmd, args)));
  useUIStore.setState({ appMode: "simple" });
  useRcloneUi.getState().closeManager();
});

describe("ConnectionPanel × rclone", () => {
  it("opens the remote manager and pre-fills a new connection from a remote", async () => {
    render(<ConnectionPanel />);
    fireEvent.click(await screen.findByText("rclone remotes"));
    const row = await screen.findByTestId("rclone-remote-al");
    fireEvent.click(within(row).getByText("Add as connection"));
    // The manager closed and the form opened as an rclone connection to al:
    await waitFor(() => expect(screen.queryByTestId("rclone-remote-manager")).toBeNull());
    const select = (await screen.findByLabelText("rclone remote")) as HTMLSelectElement;
    await waitFor(() => expect(select.value).toBe("al:"));
    expect(screen.getByDisplayValue("al")).toBeTruthy();
    // The form links back to the manager.
    fireEvent.click(screen.getByTestId("rclone-manage-remotes"));
    expect(await screen.findByTestId("rclone-remote-manager")).toBeTruthy();
  });

  it("+ Cloud offers every other provider through rclone", async () => {
    render(<ConnectionPanel />);
    fireEvent.click(await screen.findByText("+ Cloud"));
    fireEvent.click(screen.getByText(/More \(Box, pCloud, MEGA… via rclone\)/));
    expect(await screen.findByTestId("rclone-choose-type")).toBeTruthy();
  });

  it("Advanced: a crypt remote shows the banner and opens its underlying remote", async () => {
    useUIStore.setState({ appMode: "advanced" });
    render(<ConnectionPanel />);
    fireEvent.click(await screen.findByText("Secret vault"));
    await screen.findByText("plain.txt");
    const banner = await screen.findByTestId("crypt-banner");
    expect(banner).toHaveTextContent("Encrypted remote: names and contents are decrypted locally; underlying remote: /data/vault");
    expect(calls("rclone_crypt_info")).toEqual([{ remote: "sec:" }]);
    fireEvent.click(screen.getByText("Open underlying remote"));
    await screen.findByText("v0lg3ncrypt3d");
    expect(calls("connector_connect").at(-1)).toEqual(expect.objectContaining({ protocol: "rclone", host: ":local:/data/vault", connectionId: null }));
    expect(calls("connector_list_remote").at(-1)).toEqual({ protocol: "rclone", path: "/", connectionId: null });
    // The underlying remote's name heads the browser.
    expect(screen.getByText(":local:/data/vault")).toBeTruthy();
  });

  it("Advanced: sharing a file on the underlying remote sends no connection id", async () => {
    useUIStore.setState({ appMode: "advanced" });
    const policy = { enabled: true, allow_anyone: true, require_expiry: false, max_expiry_days: null, require_password: false, allow_edit: false };
    mockInvoke.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "connector_list_remote" && !args?.connectionId) return Promise.resolve([{ name: "c1pher", path: "/c1pher", is_dir: false, size: 9, modified: null }]);
      if (cmd === "sharing_policy_get") return Promise.resolve(policy);
      if (cmd === "connector_create_share_link") return Promise.resolve({ url: "https://example.test/s/1", expires_at: null, scope: "anyone", allow_edit: false, password_protected: false });
      return Promise.resolve(backend(cmd, args));
    });
    render(<ConnectionPanel />);
    fireEvent.click(await screen.findByText("Secret vault"));
    await screen.findByTestId("crypt-banner");
    fireEvent.click(screen.getByText("Open underlying remote"));
    fireEvent.click(await screen.findByTestId("remote-share-c1pher"));
    fireEvent.click(await screen.findByText("Create link"));
    await waitFor(() => expect(calls("connector_create_share_link")).toHaveLength(1));
    expect(calls("connector_create_share_link")[0]).toEqual(expect.objectContaining({ protocol: "rclone", connectionId: null, path: "/c1pher" }));
  });

  it("Standard: no crypt lookups or banner", async () => {
    render(<ConnectionPanel />);
    fireEvent.click(await screen.findByText("Secret vault"));
    await screen.findByText("plain.txt");
    expect(screen.queryByTestId("crypt-banner")).toBeNull();
    expect(calls("rclone_crypt_info")).toEqual([]);
  });
});
