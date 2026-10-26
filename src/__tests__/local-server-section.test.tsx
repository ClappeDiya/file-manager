/** WS-I row 82: Settings > Local web server and saved servers. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { LocalServerSection, type EmbeddedStatus, type ProbeResult, type ServerProfile } from "@/components/local-server-section";

let status: EmbeddedStatus;
let profiles: ServerProfile[];
let probe: (args: Record<string, unknown>) => ProbeResult;
const calls = (cmd: string) => mockInvoke.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a as Record<string, unknown>);

const stopped: EmbeddedStatus = {
  running: false,
  url: null,
  settings: { port: 8200, allow_network: false, https: false, web_root: null },
  has_password: false,
  web_interface: true,
  started_at: null,
  error: null,
};

beforeEach(() => {
  mockInvoke.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  status = { ...stopped };
  profiles = [{ name: "default", server_url: "https://nas.example.com:8200", signed_in: true, current: true }];
  probe = () => ({ ok: true, message: "ok", version: "1.4.0", uptime_secs: 3600, latency_ms: 12, untrusted_certificate: false });
  mockInvoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
    switch (cmd) {
      case "embedded_server_status":
        return status;
      case "embedded_server_set_password":
        status = { ...status, has_password: true };
        return status;
      case "embedded_server_save_settings":
        status = { ...status, settings: args.settings as EmbeddedStatus["settings"] };
        return status;
      case "embedded_server_start":
        status = { ...status, running: true, url: `http://127.0.0.1:${status.settings.port}` };
        return status;
      case "embedded_server_stop":
        status = { ...status, running: false, url: null };
        return status;
      case "server_profiles_list":
        return profiles;
      case "server_profile_save":
        profiles = [...profiles, { name: String(args.name), server_url: String(args.serverUrl), signed_in: false, current: false }];
        return profiles;
      case "server_profile_remove":
        profiles = profiles.filter((p) => p.name !== args.name);
        return profiles;
      case "server_profile_test":
        return probe(args);
      default:
        return null;
    }
  });
});

describe("Local web server", () => {
  it("cannot be started until a password is set, and the password must be confirmed", async () => {
    render(<LocalServerSection />);
    expect(await screen.findByTestId("local-server-status")).toHaveTextContent("Stopped");
    const start = screen.getByRole("button", { name: "Start server" });
    expect(start).toBeDisabled();

    const set = screen.getByRole("button", { name: "Set password" });
    fireEvent.change(screen.getByLabelText("New password"), { target: { value: "short" } });
    expect(screen.getByText("Use at least 8 characters")).toBeInTheDocument();
    expect(set).toBeDisabled();
    fireEvent.change(screen.getByLabelText("New password"), { target: { value: "correct horse battery" } });
    fireEvent.change(screen.getByLabelText("Repeat password"), { target: { value: "correct horse" } });
    expect(screen.getByText("The two passwords are different")).toBeInTheDocument();
    expect(set).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Repeat password"), { target: { value: "correct horse battery" } });
    fireEvent.click(set);

    await waitFor(() => expect(screen.getByRole("button", { name: "Start server" })).toBeEnabled());
    expect(calls("embedded_server_set_password")).toEqual([{ password: "correct horse battery" }]);
    expect(screen.getByLabelText("New password")).toHaveValue("");
  });

  it("saves the port and starts, then offers stop and open in browser", async () => {
    status = { ...stopped, has_password: true };
    render(<LocalServerSection />);
    await screen.findByTestId("local-server-status");
    fireEvent.change(screen.getByLabelText("Port"), { target: { value: "9001" } });
    fireEvent.click(screen.getByRole("button", { name: "Start server" }));

    await waitFor(() => expect(screen.getByTestId("local-server-status")).toHaveTextContent("Running at http://127.0.0.1:9001"));
    expect(calls("embedded_server_save_settings")[0]).toEqual({ settings: { port: 9001, allow_network: false, https: false, web_root: null } });
    expect(screen.getByLabelText("Port")).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: "Open in browser" }));
    await waitFor(() => expect(calls("open_server_in_browser")).toEqual([{ url: "http://127.0.0.1:9001" }]));

    fireEvent.click(screen.getByRole("button", { name: "Stop server" }));
    await waitFor(() => expect(screen.getByTestId("local-server-status")).toHaveTextContent("Stopped"));
  });

  it("shows why it could not start instead of pretending it runs", async () => {
    status = { ...stopped, has_password: true };
    const base = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
      if (cmd === "embedded_server_start") throw { message: "Port 8200 is already in use.", advice: "Choose another port." };
      return base(cmd, args);
    });
    render(<LocalServerSection />);
    await screen.findByTestId("local-server-status");
    fireEvent.click(screen.getByRole("button", { name: "Start server" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Port 8200 is already in use. Choose another port.");
    expect(screen.getByTestId("local-server-status")).toHaveTextContent("Stopped");
  });

  it("makes other computers use HTTPS", async () => {
    render(<LocalServerSection />);
    await screen.findByTestId("local-server-status");
    fireEvent.click(screen.getByLabelText("Allow other computers (uses HTTPS)"));
    const https = screen.getByLabelText("HTTPS");
    expect(https).toBeChecked();
    expect(https).toBeDisabled();
  });
});

describe("Saved servers", () => {
  it("lists the servers without tokens and tests one against its health address", async () => {
    render(<LocalServerSection />);
    const row = await screen.findByTestId("server-default");
    expect(row).toHaveTextContent("https://nas.example.com:8200");
    expect(row).toHaveTextContent("signed in from the command line");
    fireEvent.click(screen.getByRole("button", { name: "Test connection" }));
    await waitFor(() => expect(row).toHaveTextContent("Reachable: ufop-server 1.4.0, 12 ms, up 60 min"));
    expect(calls("server_profile_test")).toEqual([{ serverUrl: "https://nas.example.com:8200", acceptUntrustedCertificate: false }]);
  });

  it("reports a failed test and only accepts an untrusted certificate when asked", async () => {
    let first = true;
    probe = (args) => {
      if (first || args.acceptUntrustedCertificate !== true) {
        first = false;
        return { ok: false, message: "The server's certificate is not trusted (it may be self-signed).", version: null, uptime_secs: null, latency_ms: 5, untrusted_certificate: true };
      }
      return { ok: true, message: "ok", version: null, uptime_secs: null, latency_ms: 9, untrusted_certificate: false };
    };
    render(<LocalServerSection />);
    const row = await screen.findByTestId("server-default");
    fireEvent.click(screen.getByRole("button", { name: "Test connection" }));
    await waitFor(() => expect(row).toHaveTextContent("certificate is not trusted"));
    fireEvent.click(screen.getByLabelText(/Accept this server's untrusted certificate/));
    fireEvent.click(screen.getByRole("button", { name: "Test connection" }));
    await waitFor(() => expect(row).toHaveTextContent("Reachable"));
    expect(calls("server_profile_test").map((a) => a.acceptUntrustedCertificate)).toEqual([false, true]);
  });

  it("adds and removes servers, showing the backend's reason when an address is refused", async () => {
    render(<LocalServerSection />);
    await screen.findByTestId("server-default");
    fireEvent.change(screen.getByLabelText("Server name"), { target: { value: "office" } });
    fireEvent.change(screen.getByLabelText("Server address"), { target: { value: "https://office.example.com:8200" } });
    fireEvent.click(screen.getByRole("button", { name: "Save server" }));
    expect(await screen.findByTestId("server-office")).toHaveTextContent("https://office.example.com:8200");
    expect(calls("server_profile_save")).toEqual([{ name: "office", serverUrl: "https://office.example.com:8200" }]);

    fireEvent.click(within(screen.getByTestId("server-office")).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(screen.queryByTestId("server-office")).toBeNull());

    const base = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
      if (cmd === "server_profile_save") throw { message: "The server address must start with http:// or https://", advice: "" };
      return base(cmd, args);
    });
    fireEvent.change(screen.getByLabelText("Server name"), { target: { value: "bad" } });
    fireEvent.change(screen.getByLabelText("Server address"), { target: { value: "nas" } });
    fireEvent.click(screen.getByRole("button", { name: "Save server" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("must start with http");
  });
});
