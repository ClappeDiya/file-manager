import { describe, expect, it, vi, beforeEach, beforeAll } from "vitest";
import { render, screen, fireEvent, waitFor, within, act } from "@testing-library/react";
import { RcloneRemoteManager } from "@/components/rclone/rclone-remote-manager";
import { PromptDialogHost } from "@/components/prompt-dialog-host";
import { useUIStore } from "@/stores/ui-store";
import type { RcloneOption, RcloneProvider } from "@/lib/rclone";

const mockInvoke = vi.fn();
let authListener: ((e: { payload: unknown }) => void) | null = null;

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => mockInvoke(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (_name: string, cb: (e: { payload: unknown }) => void) => {
    authListener = cb;
    return Promise.resolve(() => {
      authListener = null;
    });
  },
}));

function opt(name: string, patch: Partial<RcloneOption> = {}): RcloneOption {
  return { name, help: `${name} help`, provider: "", default: "", examples: [], required: false, is_password: false, sensitive: false, advanced: false, exclusive: false, hidden: false, type: "string", ...patch };
}

const PROVIDERS: RcloneProvider[] = [
  { name: "alias", description: "Alias for an existing remote", prefix: "alias", hidden: false, options: [opt("remote", { required: true })] },
  {
    name: "crypt",
    description: "Encrypt/Decrypt a remote",
    prefix: "crypt",
    hidden: false,
    options: [opt("remote", { required: true }), opt("password", { required: true, is_password: true, sensitive: true }), opt("filename_encoding", { advanced: true, default: "base32" })],
  },
  { name: "box", description: "Box", prefix: "box", hidden: false, options: [opt("client_id"), opt("client_secret", { sensitive: true }), opt("box_sub_type", { advanced: true, default: "user" })] },
  { name: "google photos", description: "Google Photos", prefix: "gphotos", hidden: false, options: [] },
];

let remotes = [
  { name: "al:", backend: "alias" },
  { name: "sec:", backend: "crypt" },
];

function backend(cmd: string, args: Record<string, unknown> | undefined): unknown {
  switch (cmd) {
    case "rclone_list_remotes":
      return remotes;
    case "rclone_providers":
      return PROVIDERS;
    case "rclone_remote_create":
      if (args?.remoteType === "box") return { state: "*oauth-islocal,choose,,", option: opt("config_is_local", { type: "bool", default: "true", exclusive: true, examples: [{ value: "true", help: "Yes", provider: "" }, { value: "false", help: "No", provider: "" }] }), error: "", result: "", done: false };
      return { state: "", option: null, error: "", result: "", done: true };
    case "rclone_remote_update":
    case "rclone_config_continue":
      return { state: "", option: null, error: "", result: "", done: true };
    case "rclone_config_oauth":
      return new Promise((resolve) => setTimeout(() => resolve({ state: "", option: null, error: "", result: "", done: true }), 50));
    case "rclone_remote_get":
      return { name: "sec", type: "crypt", values: { remote: "al:vault" }, secrets_set: ["password"] };
    case "rclone_remote_test":
      return { ok: true, entries: 3, error: null, millis: 12 };
    case "rclone_cryptdecode":
      return [{ input: "abc", output: "real.txt", error: null }];
    default:
      return undefined;
  }
}

const calls = (cmd: string) => mockInvoke.mock.calls.filter((c) => c[0] === cmd).map((c) => c[1]);

beforeAll(() => {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

beforeEach(() => {
  remotes = [
    { name: "al:", backend: "alias" },
    { name: "sec:", backend: "crypt" },
  ];
  mockInvoke.mockReset();
  mockInvoke.mockImplementation((cmd: string, args?: Record<string, unknown>) => Promise.resolve(backend(cmd, args)));
  useUIStore.setState({ appMode: "simple" });
});

describe("RcloneRemoteManager", () => {
  it("lists remotes with their type, tests and deletes one", async () => {
    render(
      <>
        <RcloneRemoteManager />
        <PromptDialogHost />
      </>,
    );
    const row = await screen.findByTestId("rclone-remote-al");
    expect(within(row).getByText("alias")).toBeTruthy();
    expect(within(screen.getByTestId("rclone-remote-sec")).getByText("encrypted")).toBeTruthy();
    fireEvent.click(within(row).getByText("Test"));
    expect(await screen.findByTestId("rclone-test-ok")).toHaveTextContent("3 items");
    fireEvent.click(within(row).getByText("Delete"));
    fireEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(calls("rclone_remote_delete")).toEqual([{ name: "al" }]));
  });

  it("creates a remote from the generated form and offers Add as connection", async () => {
    const onAdd = vi.fn();
    render(<RcloneRemoteManager onAddConnection={onAdd} />);
    fireEvent.click(await screen.findByText("+ New remote"));
    // Popular: Box is installed, MEGA is not.
    const popular = await screen.findByTestId("rclone-popular");
    expect(within(popular).getByText("MEGA")).toBeDisabled();
    expect(within(popular).getByText("Box")).not.toBeDisabled();
    fireEvent.click(screen.getByText("crypt"));
    // Standard: required fields, no advanced options.
    expect(screen.getByTestId("rclone-opt-password")).toBeTruthy();
    expect(screen.queryByTestId("rclone-advanced-options")).toBeNull();
    fireEvent.change(screen.getByLabelText("Remote name"), { target: { value: "vault" } });
    fireEvent.click(screen.getByText("Create remote"));
    expect(await screen.findByText(/Fill in: remote, password/)).toBeTruthy();
    fireEvent.change(screen.getByLabelText(/^remote/), { target: { value: "al:vault" } });
    fireEvent.change(screen.getByLabelText(/^password/), { target: { value: "hunter2" } });
    fireEvent.click(screen.getByText("Create remote"));
    await screen.findByTestId("rclone-done");
    expect(calls("rclone_remote_create")).toEqual([{ name: "vault", remoteType: "crypt", params: { remote: "al:vault", password: "hunter2" }, obscure: "obscure" }]);
    fireEvent.click(screen.getByText("Add as connection"));
    expect(onAdd).toHaveBeenCalledWith("vault:", "vault");
  });

  it("shows the provider's advanced options in Advanced mode", async () => {
    useUIStore.setState({ appMode: "advanced" });
    render(<RcloneRemoteManager startInCreate initialType="crypt" />);
    const adv = await screen.findByTestId("rclone-advanced-options");
    expect(within(adv).getByTestId("rclone-opt-filename_encoding")).toBeTruthy();
    expect(screen.getByText(/already obscured/)).toBeTruthy();
  });

  it("signs in to an OAuth backend through the browser", async () => {
    const onAdd = vi.fn();
    render(<RcloneRemoteManager onAddConnection={onAdd} />);
    fireEvent.click(await screen.findByText("+ New remote"));
    fireEvent.click(within(await screen.findByTestId("rclone-popular")).getByText("Box"));
    fireEvent.click(screen.getByText("Create remote"));
    fireEvent.click(await screen.findByText("Sign in with browser"));
    await screen.findByTestId("rclone-signing-in");
    await waitFor(() => expect(authListener).not.toBeNull());
    act(() => authListener?.({ payload: { name: "box", url: "http://127.0.0.1:53682/auth?state=x", opened: true } }));
    expect(await screen.findByText("Open the sign-in page")).toHaveAttribute("href", "http://127.0.0.1:53682/auth?state=x");
    const done = await screen.findByTestId("rclone-done");
    expect(calls("rclone_config_oauth")).toEqual([{ name: "box", state: "*oauth-islocal,choose,,", params: {}, openBrowser: true, obscure: "obscure" }]);
    // The remote was created through a question: the done screen still says so and offers Add as connection.
    expect(done).toHaveTextContent("was created");
    fireEvent.click(within(done).getByText("Add as connection"));
    expect(onAdd).toHaveBeenCalledWith("box:", "box");
  });

  it("answers rclone's questions manually when asked to", async () => {
    const onAdd = vi.fn();
    render(<RcloneRemoteManager onAddConnection={onAdd} />);
    fireEvent.click(await screen.findByText("+ New remote"));
    fireEvent.click(within(await screen.findByTestId("rclone-popular")).getByText("Box"));
    fireEvent.click(screen.getByText("Create remote"));
    fireEvent.click(await screen.findByText("Answer rclone's questions manually"));
    fireEvent.change(screen.getByLabelText("config_is_local"), { target: { value: "false" } });
    fireEvent.click(screen.getByText("Continue"));
    const done = await screen.findByTestId("rclone-done");
    expect(calls("rclone_config_continue")).toEqual([{ name: "box", state: "*oauth-islocal,choose,,", result: "false", params: {}, obscure: "obscure" }]);
    expect(done).toHaveTextContent("was created");
    fireEvent.click(within(done).getByText("Add as connection"));
    expect(onAdd).toHaveBeenCalledWith("box:", "box");
  });

  it("keeps a new remote's Discard and Add as connection across follow-up questions, and masks sensitive answers", async () => {
    const onAdd = vi.fn();
    mockInvoke.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "rclone_config_continue" && args?.state === "*oauth-islocal,choose,,")
        return Promise.resolve({ state: "*oauth-authorize,,", option: opt("config_token", { help: "Paste the result of rclone authorize", required: true, sensitive: true }), error: "", result: "", done: false });
      return Promise.resolve(backend(cmd, args));
    });
    render(
      <>
        <RcloneRemoteManager onAddConnection={onAdd} />
        <PromptDialogHost />
      </>,
    );
    fireEvent.click(await screen.findByText("+ New remote"));
    fireEvent.click(within(await screen.findByTestId("rclone-popular")).getByText("Box"));
    fireEvent.click(screen.getByText("Create remote"));
    expect(await screen.findByText("Discard this remote")).toBeTruthy();
    fireEvent.click(await screen.findByText("Answer rclone's questions manually"));
    fireEvent.change(screen.getByLabelText("config_is_local"), { target: { value: "false" } });
    fireEvent.click(screen.getByText("Continue"));
    // Second question of the same new remote: Discard is still offered, the token is masked.
    const token = (await screen.findByLabelText(/^config_token/)) as HTMLInputElement;
    expect(token.type).toBe("password");
    expect(screen.getByText("Discard this remote")).toBeTruthy();
    fireEvent.change(token, { target: { value: '{"access_token":"x"}' } });
    fireEvent.click(screen.getByText("Continue"));
    const done = await screen.findByTestId("rclone-done");
    expect(done).toHaveTextContent("was created");
    fireEvent.click(within(done).getByText("Add as connection"));
    expect(onAdd).toHaveBeenCalledWith("box:", "box");
  });

  it("carries the create call's obscure mode through follow-up questions", async () => {
    // A follow-up step re-sends the plain password: without the flag rclone
    // may store a base64-looking one raw over the obscured value.
    mockInvoke.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "rclone_remote_create" && args?.remoteType === "crypt")
        return Promise.resolve({ state: "*2fa", option: opt("config_2fa", { help: "Two-factor code", required: true }), error: "", result: "", done: false });
      return Promise.resolve(backend(cmd, args));
    });
    render(<RcloneRemoteManager startInCreate initialType="crypt" />);
    fireEvent.change(await screen.findByLabelText(/^Remote name/), { target: { value: "vault" } });
    fireEvent.change(screen.getByLabelText(/^remote/), { target: { value: "al:vault" } });
    fireEvent.change(screen.getByLabelText(/^password/), { target: { value: "abcdefghijklmnopqrstuvwxyz012345" } });
    fireEvent.click(screen.getByText("Create remote"));
    fireEvent.change(await screen.findByLabelText(/^config_2fa/), { target: { value: "123456" } });
    fireEvent.click(screen.getByText("Continue"));
    await screen.findByTestId("rclone-done");
    expect(calls("rclone_config_continue")).toEqual([
      { name: "vault", state: "*2fa", result: "123456", params: { remote: "al:vault", password: "abcdefghijklmnopqrstuvwxyz012345" }, obscure: "obscure" },
    ]);
  });

  it("keeps an edit's already-obscured mode for the sign-in that follows", async () => {
    useUIStore.setState({ appMode: "advanced" });
    mockInvoke.mockImplementation((cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "rclone_remote_update")
        return Promise.resolve({ state: "*oauth-islocal,,", option: opt("config_is_local", { type: "bool", default: "true", exclusive: true }), error: "", result: "", done: false });
      return Promise.resolve(backend(cmd, args));
    });
    render(<RcloneRemoteManager />);
    fireEvent.click(within(await screen.findByTestId("rclone-remote-sec")).getByText("Edit"));
    fireEvent.change(await screen.findByLabelText(/^password/), { target: { value: "x6xLnMK3obscured" } });
    fireEvent.click(screen.getByText("New passwords are already obscured"));
    fireEvent.click(screen.getByText("Save"));
    fireEvent.click(await screen.findByText("Sign in with browser"));
    await screen.findByTestId("rclone-done");
    expect(calls("rclone_remote_update")).toEqual([{ name: "sec", params: { password: "x6xLnMK3obscured" }, obscure: "already_obscured" }]);
    expect(calls("rclone_config_oauth")).toEqual([{ name: "sec", state: "*oauth-islocal,,", params: { password: "x6xLnMK3obscured" }, openBrowser: true, obscure: "already_obscured" }]);
  });

  it("does not offer Add as connection after editing a remote", async () => {
    render(<RcloneRemoteManager onAddConnection={vi.fn()} />);
    fireEvent.click(within(await screen.findByTestId("rclone-remote-sec")).getByText("Edit"));
    fireEvent.change(await screen.findByLabelText(/^remote/), { target: { value: "al:vault2" } });
    fireEvent.click(screen.getByText("Save"));
    const done = await screen.findByTestId("rclone-done");
    expect(done).toHaveTextContent("was updated");
    expect(within(done).queryByText("Add as connection")).toBeNull();
  });

  it("says rclone is not working instead of 'No rclone remotes yet' when rclone cannot start", async () => {
    const notStarted = { message: "Could not start rclone: No such file or directory (os error 2)", advice: "Check the rclone binary in Settings → rclone." };
    mockInvoke.mockImplementation((cmd: string, args?: Record<string, unknown>) =>
      cmd === "rclone_list_remotes" || cmd === "rclone_providers" ? Promise.reject(notStarted) : Promise.resolve(backend(cmd, args)),
    );
    render(<RcloneRemoteManager />);
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent("rclone is not installed or not working — Settings → rclone");
    expect(alert).toHaveTextContent("Could not start rclone");
    expect(screen.queryByText(/No rclone remotes yet/)).toBeNull();
    await waitFor(() => expect(screen.getByText("+ New remote")).toBeDisabled());
  });

  it("shows rclone's error when the remotes cannot be listed", async () => {
    mockInvoke.mockImplementation((cmd: string, args?: Record<string, unknown>) =>
      cmd === "rclone_list_remotes"
        ? Promise.reject({ message: "rclone: Failed to load config file: unable to decrypt configuration", advice: "Check the remote with `rclone config` and try again." })
        : Promise.resolve(backend(cmd, args)),
    );
    render(<RcloneRemoteManager />);
    expect(await screen.findByRole("alert")).toHaveTextContent("unable to decrypt configuration");
    expect(screen.queryByText(/No rclone remotes yet/)).toBeNull();
    // The providers loaded, so a new remote can still be started.
    await waitFor(() => expect(screen.getByText("+ New remote")).not.toBeDisabled());
  });

  it("edits a remote without sending stored secrets back", async () => {
    render(<RcloneRemoteManager />);
    fireEvent.click(within(await screen.findByTestId("rclone-remote-sec")).getByText("Edit"));
    const remote = (await screen.findByLabelText(/^remote/)) as HTMLInputElement;
    expect(remote.value).toBe("al:vault");
    expect(screen.getByText(/stored; leave blank to keep it/)).toBeTruthy();
    fireEvent.change(remote, { target: { value: "al:vault2" } });
    fireEvent.click(screen.getByText("Save"));
    await screen.findByTestId("rclone-done");
    expect(calls("rclone_remote_update")).toEqual([{ name: "sec", params: { remote: "al:vault2" }, obscure: "obscure" }]);
  });

  it("decodes crypt names from the list in Advanced mode", async () => {
    useUIStore.setState({ appMode: "advanced" });
    render(<RcloneRemoteManager />);
    fireEvent.click(within(await screen.findByTestId("rclone-remote-sec")).getByText("Names…"));
    fireEvent.change(screen.getByLabelText("Names, one per line"), { target: { value: "abc\n" } });
    fireEvent.click(screen.getByText("Decrypt names"));
    expect(await screen.findByTestId("crypt-results")).toHaveTextContent("real.txt");
    expect(calls("rclone_cryptdecode")).toEqual([{ remote: "sec:", names: ["abc"], reverse: false }]);
  });
});
