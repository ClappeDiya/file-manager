/**
 * Cloud connect wizards (guided flows) use the real commands: the
 * connection is saved, signed in, its location picked from the account,
 * and connected + listed before Finish. (They used to only flip state.)
 */
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { ConnectGDriveWizard, ConnectGCSWizard, ConnectOneDriveWizard } from "@/components/guided-flows/cloud-connect-flows";
import { useUIStore } from "@/stores/ui-store";

const calls = (cmd: string) => mockInvoke.mock.calls.filter(([c]) => c === cmd).map(([, a]) => a);

function backend(over: Record<string, (a: Record<string, unknown>) => unknown> = {}) {
  mockInvoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
    if (over[cmd]) return over[cmd](args);
    switch (cmd) {
      case "save_connection":
        return { id: "c1", name: args.name, protocol: args.protocol };
      case "oauth_provider_info":
        return { provider: "google_drive", label: "Google Drive", has_builtin_client: true, uses_client_secret: true, redirect_uri: "http://localhost:9876/callback", app_console: "x" };
      case "oauth_begin":
        return { state: "s1", provider: "google_drive", auth_url: "https://accounts.google.com/x", redirect_uri: "http://localhost:9876/callback", browser_opened: true };
      case "oauth_await":
        return { connection_id: "c1", provider: "google_drive", account: "ada@example.com" };
      case "gdrive_list_shared_drives":
        return [{ id: "D1", name: "Team" }];
      case "connector_connect":
        return undefined;
      case "connector_list_remote":
        return [{ name: "Reports", path: "/shared-drives/D1/f1", is_dir: true, size: 0 }];
      default:
        return undefined;
    }
  });
}

beforeAll(() => {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

beforeEach(() => {
  useUIStore.setState({ appMode: "simple", activeGuidedFlow: "connect-gdrive" });
  mockInvoke.mockReset();
  backend();
});

const next = () => fireEvent.click(screen.getByRole("button", { name: /^(Next|Finish|Connect)$/ }));

describe("Google Drive wizard", () => {
  it("saves, signs in, picks a shared drive, lists it and finishes", async () => {
    const changed = vi.fn();
    window.addEventListener("ufop:connections-changed", changed);
    render(<ConnectGDriveWizard />);
    fireEvent.change(screen.getByLabelText("Connection name"), { target: { value: "Team Drive" } });
    next();
    await waitFor(() => expect(calls("save_connection")).toHaveLength(1));
    expect(calls("save_connection")[0]).toMatchObject({ name: "Team Drive", protocol: "google_drive", connectionId: null, password: null });

    // Can't skip the sign-in.
    fireEvent.click(await screen.findByRole("button", { name: "Sign in with Google Drive" }));
    expect(await screen.findByTestId("cloud-signed-in")).toHaveTextContent("ada@example.com");
    expect(calls("oauth_begin")[0]).toMatchObject({ connectionId: "c1" });
    next();

    fireEvent.change(await screen.findByLabelText("Open"), { target: { value: "shared_drive" } });
    fireEvent.click(screen.getByRole("button", { name: "Load" }));
    const picker = (await screen.findByLabelText("Shared drive")) as HTMLSelectElement;
    await waitFor(() => expect([...picker.options].map((o) => o.textContent)).toContain("Team"));
    fireEvent.change(picker, { target: { value: "D1" } });
    // Finishing without checking the location is refused.
    next();
    expect(await screen.findByText(/Check the location/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "List it" }));
    expect(await screen.findByTestId("cloud-listing")).toHaveTextContent("Reports");
    expect(calls("connector_connect").at(-1)).toMatchObject({ protocol: "google_drive", connectionId: "c1" });
    next();
    await waitFor(() => expect(useUIStore.getState().activeGuidedFlow).toBeNull());
    const last = calls("save_connection").at(-1) as { connectionId: string; extras: { google_drive: unknown } };
    expect(last.connectionId).toBe("c1");
    expect(last.extras.google_drive).toEqual({ root: "shared_drive", shared_drive_id: "D1", shared_drive_name: "Team", export_formats: { document: null, spreadsheet: null, presentation: null, drawing: null } });
    expect(changed).toHaveBeenCalled();
    window.removeEventListener("ufop:connections-changed", changed);
  });

  it("does not continue when the sign-in has not happened", async () => {
    render(<ConnectGDriveWizard />);
    next();
    await screen.findByRole("button", { name: "Sign in with Google Drive" });
    next();
    expect(await screen.findByText("Sign in to Google Drive to continue.")).toBeInTheDocument();
    expect(calls("oauth_begin")).toHaveLength(0);
  });
});

describe("OneDrive wizard", () => {
  it("saves the work-account choice before signing in (it picks the Microsoft endpoint)", async () => {
    render(<ConnectOneDriveWizard />);
    fireEvent.change(screen.getByLabelText("Account"), { target: { value: "business" } });
    next();
    await waitFor(() => expect(calls("save_connection")).toHaveLength(1));
    expect((calls("save_connection")[0] as { extras: { oauth: { account_kind: string } } }).extras.oauth.account_kind).toBe("business");
  });
});

describe("Google Cloud Storage wizard", () => {
  it("tests by saving the connection and listing its buckets, and shows real failures", async () => {
    useUIStore.setState({ activeGuidedFlow: "connect-gcs" });
    backend({
      connector_connect: () => {
        throw { message: "Sign-in rejected: SignatureDoesNotMatch", advice: "Check the keys." };
      },
    });
    render(<ConnectGCSWizard />);
    next();
    fireEvent.change(screen.getByLabelText("Access Key ID"), { target: { value: "GOOG1EKEY" } });
    fireEvent.change(screen.getByLabelText("Secret Access Key"), { target: { value: "secret" } });
    next();
    fireEvent.click(await screen.findByRole("button", { name: "Test Connection" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("SignatureDoesNotMatch");
    expect(calls("save_connection")[0]).toMatchObject({ protocol: "s3", host: "https://storage.googleapis.com", username: "GOOG1EKEY", password: "secret" });
    backend();
    fireEvent.click(screen.getByRole("button", { name: "Try Again" }));
    expect(await screen.findByTestId("gcs-success")).toHaveTextContent("Listed 1 bucket");
    next();
    await waitFor(() => expect(useUIStore.getState().activeGuidedFlow).toBeNull());
  });
});
