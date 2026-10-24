/** OAuth sign-in for Google Drive / Dropbox / OneDrive connections (item 24). */
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { OAuthSignIn } from "@/components/oauth-sign-in";
import { useUIStore } from "@/stores/ui-store";

const INFO = {
  provider: "google_drive",
  label: "Google Drive",
  has_builtin_client: true,
  uses_client_secret: true,
  redirect_uri: "http://localhost:9876/callback",
  app_console: "https://console.cloud.google.com/apis/credentials",
};
const START = { state: "st-1", provider: "google_drive", auth_url: "https://accounts.google.com/o/oauth2/v2/auth?x=1", redirect_uri: INFO.redirect_uri, browser_opened: true };
const SIGNED = { connection_id: "c1", provider: "google_drive", account: "ada@example.com" };

const calls = (cmd: string) => mockInvoke.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a);

beforeAll(() => {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

beforeEach(() => {
  useUIStore.setState({ appMode: "simple" });
  mockInvoke.mockReset();
});

function backend(overrides: Record<string, (args: Record<string, unknown>) => unknown> = {}) {
  mockInvoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
    if (overrides[cmd]) return overrides[cmd](args);
    if (cmd === "oauth_provider_info") return INFO;
    if (cmd === "oauth_begin") return START;
    if (cmd === "oauth_await") return SIGNED;
    if (cmd === "oauth_cancel") return undefined;
    return undefined;
  });
}

describe("OAuthSignIn", () => {
  it("signs in through the browser with the built-in app and reports the account", async () => {
    backend();
    const done = vi.fn();
    render(<OAuthSignIn connectionId="c1" protocol="google_drive" onSignedIn={done} />);
    fireEvent.click(await screen.findByRole("button", { name: "Sign in with Google Drive" }));
    await waitFor(() => expect(done).toHaveBeenCalledWith(SIGNED));
    expect(calls("oauth_begin")).toEqual([{ connectionId: "c1", clientId: null, clientSecret: null, openBrowser: true }]);
    expect(calls("oauth_await")).toEqual([{ state: "st-1" }]);
  });

  it("says plainly when this build has no built-in app, and offers Advanced view", async () => {
    backend({ oauth_provider_info: () => ({ ...INFO, has_builtin_client: false }) });
    render(<OAuthSignIn connectionId="c1" protocol="google_drive" onSignedIn={vi.fn()} />);
    expect(await screen.findByTestId("oauth-no-builtin")).toHaveTextContent("no built-in Google Drive app");
    expect(screen.getByRole("button", { name: "Sign in with Google Drive" })).toBeDisabled();
    // The custom app fields are Advanced only.
    expect(screen.queryByLabelText("Client ID")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Switch to Advanced view" }));
    expect(useUIStore.getState().appMode).toBe("advanced");
    fireEvent.change(await screen.findByLabelText("Client ID"), { target: { value: " my-app.apps.googleusercontent.com " } });
    fireEvent.change(screen.getByLabelText("Client secret"), { target: { value: "s3cr3t" } });
    expect(screen.getByText(INFO.redirect_uri)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Sign in with Google Drive" }));
    await waitFor(() => expect(calls("oauth_begin")).toEqual([{ connectionId: "c1", clientId: "my-app.apps.googleusercontent.com", clientSecret: "s3cr3t", openBrowser: true }]));
  });

  it("finishes with a code pasted from a browser on another device", async () => {
    let resolveAwait: (v: unknown) => void = () => {};
    backend({
      oauth_await: () => new Promise((r) => (resolveAwait = r)),
      oauth_complete: () => SIGNED,
    });
    const done = vi.fn();
    render(<OAuthSignIn connectionId="c1" protocol="google_drive" onSignedIn={done} />);
    fireEvent.click(await screen.findByRole("button", { name: "Sign in with Google Drive" }));
    expect(await screen.findByTestId("oauth-waiting")).toBeInTheDocument();
    fireEvent.change(screen.getByLabelText(/Paste the code/), { target: { value: "http://localhost:9876/callback?code=abc&state=st-1" } });
    fireEvent.click(screen.getByRole("button", { name: "Use code" }));
    await waitFor(() => expect(done).toHaveBeenCalledWith(SIGNED));
    expect(calls("oauth_complete")).toEqual([{ state: "st-1", code: "http://localhost:9876/callback?code=abc&state=st-1" }]);
    await act(async () => resolveAwait(SIGNED));
  });

  it("a pasted code that finishes the sign-in is not followed by an error or a second finish", async () => {
    // Regression: when the pasted code finished the sign-in, the browser wait
    // ended (rejected, or resolved late) and showed an error / finished twice.
    for (const ending of ["rejects", "resolves"] as const) {
      mockInvoke.mockReset();
      let endAwait: () => void = () => {};
      backend({
        oauth_await: () =>
          new Promise((resolve, reject) => {
            endAwait = () => (ending === "rejects" ? reject({ message: "No sign-in is waiting for that state", advice: "Start again." }) : resolve(SIGNED));
          }),
        oauth_complete: () => SIGNED,
      });
      const done = vi.fn();
      const { unmount } = render(<OAuthSignIn connectionId="c1" protocol="google_drive" onSignedIn={done} />);
      fireEvent.click(await screen.findByRole("button", { name: "Sign in with Google Drive" }));
      fireEvent.change(await screen.findByLabelText(/Paste the code/), { target: { value: "abc" } });
      fireEvent.click(screen.getByRole("button", { name: "Use code" }));
      await waitFor(() => expect(done).toHaveBeenCalledTimes(1));
      await act(async () => endAwait());
      expect(done).toHaveBeenCalledTimes(1);
      expect(screen.queryByTestId("oauth-error")).toBeNull();
      unmount();
    }
  });

  it("the browser wait ending while a pasted code is still being exchanged is not an error", async () => {
    // Regression: the wait rejected ("finished another way") before the
    // pasted code's exchange returned, so the panel showed an error and
    // reset to "Sign in"; the success arrived afterwards.
    let rejectAwait: () => void = () => {};
    let resolveComplete: () => void = () => {};
    backend({
      oauth_await: () => new Promise((_, reject) => (rejectAwait = () => reject({ message: "The sign-in was cancelled or finished another way", advice: "" }))),
      oauth_complete: () => new Promise((r) => (resolveComplete = () => r(SIGNED))),
    });
    const done = vi.fn();
    render(<OAuthSignIn connectionId="c1" protocol="google_drive" onSignedIn={done} />);
    fireEvent.click(await screen.findByRole("button", { name: "Sign in with Google Drive" }));
    fireEvent.change(await screen.findByLabelText(/Paste the code/), { target: { value: "abc" } });
    fireEvent.click(screen.getByRole("button", { name: "Use code" }));
    await waitFor(() => expect(calls("oauth_complete")).toHaveLength(1));
    await act(async () => rejectAwait());
    expect(screen.queryByTestId("oauth-error")).toBeNull();
    expect(screen.getByTestId("oauth-waiting")).toBeInTheDocument();
    await act(async () => resolveComplete());
    await waitFor(() => expect(done).toHaveBeenCalledTimes(1));
    expect(done).toHaveBeenCalledWith(SIGNED);
    expect(screen.queryByTestId("oauth-error")).toBeNull();
  });

  it("a mistyped pasted code can be corrected without starting over", async () => {
    backend({
      oauth_await: () => new Promise(() => {}),
      oauth_complete: (a) => {
        if (a.code === "typo") throw { message: "The code was refused (invalid_grant)", advice: "Paste it again." };
        return SIGNED;
      },
    });
    const done = vi.fn();
    render(<OAuthSignIn connectionId="c1" protocol="google_drive" onSignedIn={done} />);
    fireEvent.click(await screen.findByRole("button", { name: "Sign in with Google Drive" }));
    fireEvent.change(await screen.findByLabelText(/Paste the code/), { target: { value: "typo" } });
    fireEvent.click(screen.getByRole("button", { name: "Use code" }));
    expect(await screen.findByTestId("oauth-error")).toHaveTextContent("invalid_grant");
    fireEvent.change(screen.getByLabelText(/Paste the code/), { target: { value: "good" } });
    fireEvent.click(screen.getByRole("button", { name: "Use code" }));
    await waitFor(() => expect(done).toHaveBeenCalledTimes(1));
    expect(calls("oauth_begin")).toHaveLength(1);
  });

  it("cancels a waiting sign-in", async () => {
    backend({ oauth_await: () => new Promise(() => {}) });
    render(<OAuthSignIn connectionId="c1" protocol="google_drive" onSignedIn={vi.fn()} />);
    fireEvent.click(await screen.findByRole("button", { name: "Sign in with Google Drive" }));
    fireEvent.click(await screen.findByRole("button", { name: "Cancel sign-in" }));
    await waitFor(() => expect(calls("oauth_cancel")).toEqual([{ state: "st-1" }]));
    expect(screen.getByRole("button", { name: "Sign in with Google Drive" })).toBeInTheDocument();
  });

  it("signs in a headless computer with pasted tokens or a broker AuthID", async () => {
    backend({ oauth_import_grant: () => SIGNED });
    const done = vi.fn();
    render(<OAuthSignIn connectionId="c1" protocol="google_drive" onSignedIn={done} />);
    fireEvent.click(await screen.findByText("Sign in on a computer without a browser"));
    fireEvent.change(screen.getByLabelText("Token"), { target: { value: '{"refresh_token":"r","client_id":"c"}' } });
    fireEvent.click(screen.getByRole("button", { name: "Check and save" }));
    await waitFor(() => expect(done).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByLabelText("AuthID from a sign-in broker"));
    fireEvent.change(screen.getByLabelText("Broker URL"), { target: { value: "https://ufop.example.com" } });
    fireEvent.change(screen.getByLabelText("AuthID"), { target: { value: "google:abc" } });
    fireEvent.click(screen.getByRole("button", { name: "Check and save" }));
    await waitFor(() => expect(done).toHaveBeenCalledTimes(2));
    expect(calls("oauth_import_grant")).toEqual([
      { connectionId: "c1", token: '{"refresh_token":"r","client_id":"c"}', brokerUrl: null, authid: null },
      { connectionId: "c1", token: null, brokerUrl: "https://ufop.example.com", authid: "google:abc" },
    ]);
  });

  it("shows why a sign-in failed instead of pretending", async () => {
    backend({
      oauth_await: () => {
        throw { message: "Renewing the sign-in was refused (HTTP 400: invalid_grant)", advice: "Sign in again." };
      },
    });
    const done = vi.fn();
    render(<OAuthSignIn connectionId="c1" protocol="google_drive" onSignedIn={done} />);
    fireEvent.click(await screen.findByRole("button", { name: "Sign in with Google Drive" }));
    expect(await screen.findByTestId("oauth-error")).toHaveTextContent("invalid_grant");
    expect(done).not.toHaveBeenCalled();
  });
});
