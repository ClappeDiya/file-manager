import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";

const mockInvoke = vi.fn();
let lockListener: ((e: { payload: { locked: boolean } }) => void) | null = null;
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (_name: string, cb: (e: { payload: { locked: boolean } }) => void) => {
    lockListener = cb;
    return () => {
      lockListener = null;
    };
  },
}));

import { setIpcLocked, tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import { AppLockGate } from "@/components/app-lock-screen";
import { AppLockSection } from "@/components/app-lock-section";
import { useAppLockStore, type AppLockStatus } from "@/stores/app-lock-store";

let status: AppLockStatus;
const PASSWORD = "correct horse";

beforeEach(() => {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  setIpcLocked(false);
  lockListener = null;
  useAppLockStore.setState({ status: null });
  status = { enabled: true, locked: false, lock_on_start: true, idle_minutes: 5, has_password: true };
  mockInvoke.mockReset().mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    switch (cmd) {
      case "app_lock_status":
        return status;
      case "app_lock_unlock":
        if (args?.password === PASSWORD) status = { ...status, locked: false };
        return args?.password === PASSWORD;
      case "app_lock_now":
        status = { ...status, locked: true };
        return null;
      case "app_lock_set_settings": {
        const s = args!.settings as { enabled: boolean; lock_on_start: boolean; idle_minutes: number };
        status = { ...status, ...s, has_password: status.has_password || !!args!.newPassword };
        return status;
      }
      case "list_directory":
        return [`listing of ${String(args?.path)}`];
      default:
        return null;
    }
  });
});

const calls = (cmd: string) => mockInvoke.mock.calls.filter((c) => c[0] === cmd);

describe("app lock: commands wait while locked", () => {
  it("holds calls until the unlock, runs identical ones once, lets the lock screen's through", async () => {
    setIpcLocked(true);
    const a = tauriInvoke<string[]>("list_directory", { path: "/home" });
    const b = tauriInvokeSafe<string[]>("list_directory", { path: "/home" }, []);
    const c = tauriInvoke<string[]>("list_directory", { path: "/tmp" });
    expect(await tauriInvoke("app_lock_status")).toEqual(status);
    await new Promise((r) => setTimeout(r, 10));
    expect(calls("list_directory")).toHaveLength(0);
    setIpcLocked(false);
    expect(await a).toEqual(["listing of /home"]);
    expect(await b).toEqual(["listing of /home"]);
    expect(await c).toEqual(["listing of /tmp"]);
    expect(calls("list_directory")).toHaveLength(2);
  });

  it("a command the backend refused as locked runs again after the unlock", async () => {
    status = { ...status, locked: true };
    let refuse = true;
    const base = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "list_directory" && refuse) {
        refuse = false;
        throw { error_type: "app_locked", message: "UFOP is locked", advice: "" };
      }
      return base(cmd, args);
    });
    const p = tauriInvoke<string[]>("list_directory", { path: "/x" });
    await new Promise((r) => setTimeout(r, 10));
    expect(calls("list_directory")).toHaveLength(1);
    setIpcLocked(false);
    expect(await p).toEqual(["listing of /x"]);
  });
});

describe("app lock: the lock screen", () => {
  it("covers a locked window, refuses a wrong password and unlocks with the right one", async () => {
    status = { ...status, locked: true };
    render(
      <AppLockGate>
        <button>behind</button>
      </AppLockGate>,
    );
    expect(await screen.findByTestId("app-lock-screen")).toBeInTheDocument();
    expect(screen.getByTestId("app-lock-content")).toHaveAttribute("inert");
    fireEvent.change(screen.getByTestId("app-lock-password"), { target: { value: "wrong" } });
    fireEvent.click(screen.getByTestId("app-lock-unlock"));
    expect(await screen.findByTestId("app-lock-error")).toHaveTextContent("Wrong password.");
    fireEvent.change(screen.getByTestId("app-lock-password"), { target: { value: PASSWORD } });
    fireEvent.click(screen.getByTestId("app-lock-unlock"));
    await waitFor(() => expect(screen.queryByTestId("app-lock-screen")).toBeNull());
    expect(screen.getByTestId("app-lock-content")).not.toHaveAttribute("inert");
  });

  it("shows why the password manager refuses attempts", async () => {
    status = { ...status, locked: true };
    const base = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "app_lock_unlock") throw { message: "Too many failed attempts. Try again in 5 minutes", advice: "" };
      return base(cmd, args);
    });
    render(<AppLockGate>x</AppLockGate>);
    fireEvent.change(await screen.findByTestId("app-lock-password"), { target: { value: "a" } });
    fireEvent.click(screen.getByTestId("app-lock-unlock"));
    expect(await screen.findByTestId("app-lock-error")).toHaveTextContent(/Too many failed attempts/);
  });

  it("locks when the backend says so (idle, tray, another window)", async () => {
    render(<AppLockGate>content</AppLockGate>);
    await waitFor(() => expect(lockListener).not.toBeNull());
    await waitFor(() => expect(useAppLockStore.getState().status).not.toBeNull());
    act(() => lockListener!({ payload: { locked: true } }));
    expect(screen.getByTestId("app-lock-screen")).toBeInTheDocument();
  });

  it("reports use of the window at most every 20 seconds while an idle limit is set", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    render(<AppLockGate>content</AppLockGate>);
    await waitFor(() => expect(useAppLockStore.getState().status).not.toBeNull());
    fireEvent.keyDown(window, { key: "a" });
    fireEvent.pointerDown(window);
    expect(calls("app_lock_activity")).toHaveLength(1);
    vi.advanceTimersByTime(21_000);
    fireEvent.keyDown(window, { key: "b" });
    expect(calls("app_lock_activity")).toHaveLength(2);
    vi.useRealTimers();
  });
});

describe("app lock: review fixes", () => {
  // Review finding: a call refused as locked set only the IPC flag; the lock
  // screen never showed, and the window waited forever.
  it("a call refused as locked shows the lock screen even when the event was missed", async () => {
    render(<AppLockGate>content</AppLockGate>);
    await waitFor(() => expect(useAppLockStore.getState().status?.locked).toBe(false));
    // Locked by the idle timer; this window never got the event.
    status = { ...status, locked: true };
    const base = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "list_directory" && status.locked) throw { error_type: "app_locked", message: "UFOP is locked", advice: "" };
      return base(cmd, args);
    });
    const p = tauriInvoke<string[]>("list_directory", { path: "/y" });
    expect(await screen.findByTestId("app-lock-screen")).toBeInTheDocument();
    fireEvent.change(screen.getByTestId("app-lock-password"), { target: { value: PASSWORD } });
    fireEvent.click(screen.getByTestId("app-lock-unlock"));
    expect(await p).toEqual(["listing of /y"]);
  });

  // Review finding: with lock-on-start the app showed (tabs, favorites)
  // until the state arrived.
  it("shows nothing of the app until the lock state is known", async () => {
    let answer: (s: AppLockStatus) => void = () => {};
    const base = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) =>
      cmd === "app_lock_status" ? new Promise((r) => (answer = r)) : base(cmd, args),
    );
    render(<AppLockGate>secret tabs</AppLockGate>);
    expect(screen.getByTestId("app-lock-content")).toHaveClass("invisible");
    expect(screen.getByTestId("app-lock-content")).toHaveAttribute("inert");
    await waitFor(() => expect(calls("app_lock_status").length).toBeGreaterThan(0));
    act(() => answer({ ...status, locked: true }));
    expect(await screen.findByTestId("app-lock-screen")).toBeInTheDocument();
  });

  it("an unanswered status never hides the app for good", async () => {
    const base = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "app_lock_status") throw new Error("unknown command");
      return base(cmd, args);
    });
    render(<AppLockGate>visible</AppLockGate>);
    await waitFor(() => expect(screen.getByTestId("app-lock-content")).not.toHaveClass("invisible"));
  });
});

describe("app lock: Settings", () => {
  it("turning it on the first time creates the master password", async () => {
    status = { enabled: false, locked: false, lock_on_start: true, idle_minutes: 0, has_password: false };
    render(<AppLockSection />);
    fireEvent.click(await screen.findByTestId("app-lock-enabled"));
    fireEvent.change(screen.getByTestId("app-lock-idle"), { target: { value: "15" } });
    fireEvent.change(screen.getByTestId("app-lock-new-password"), { target: { value: "short" } });
    expect(screen.getByTestId("app-lock-save")).toBeDisabled();
    expect(screen.getByText(/at least 8 characters/)).toBeInTheDocument();
    fireEvent.change(screen.getByTestId("app-lock-new-password"), { target: { value: "long enough" } });
    fireEvent.change(screen.getByTestId("app-lock-confirm"), { target: { value: "long enough" } });
    fireEvent.click(screen.getByTestId("app-lock-save"));
    await waitFor(() => expect(screen.getByText("App lock saved.")).toBeInTheDocument());
    expect(calls("app_lock_set_settings")[0][1]).toEqual({ settings: { enabled: true, lock_on_start: true, idle_minutes: 15 }, password: null, newPassword: "long enough" });
  });

  it("changing or turning off an active lock asks for the master password; Lock now locks", async () => {
    render(<AppLockSection />);
    fireEvent.click(await screen.findByTestId("app-lock-now"));
    await waitFor(() => expect(calls("app_lock_now")).toHaveLength(1));
    act(() => useAppLockStore.getState().setLocked(false));
    fireEvent.click(screen.getByTestId("app-lock-enabled"));
    expect(screen.getByTestId("app-lock-save")).toBeDisabled();
    fireEvent.change(screen.getByTestId("app-lock-password-field"), { target: { value: PASSWORD } });
    fireEvent.click(screen.getByTestId("app-lock-save"));
    await waitFor(() => expect(screen.getByText("App lock turned off.")).toBeInTheDocument());
    expect(calls("app_lock_set_settings")[0][1]).toMatchObject({ settings: { enabled: false }, password: PASSWORD, newPassword: null });
  });
});
