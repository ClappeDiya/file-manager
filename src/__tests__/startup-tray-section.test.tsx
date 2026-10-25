import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { StartupTraySection, type StartupSettings } from "@/components/startup-tray-section";

const BASE: StartupSettings = {
  start_at_login: false,
  start_minimized: false,
  start_hidden_at_login: true,
  close_to_tray: true,
  minimize_to_tray: true,
  show_tray_icon: true,
};

let stored: StartupSettings;

beforeEach(() => {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  stored = { ...BASE };
  mockInvoke.mockReset().mockImplementation(async (cmd: string, args?: { settings: StartupSettings }) => {
    if (cmd === "get_startup_settings") return stored;
    if (cmd === "set_startup_settings") return (stored = { ...args!.settings });
    return null;
  });
});

const box = (id: string) => screen.getByTestId(id) as HTMLInputElement;

describe("Settings → Startup & Tray", () => {
  it("shows the saved choices and saves a change", async () => {
    render(<StartupTraySection />);
    await waitFor(() => expect(box("startup-close-to-tray").checked).toBe(true));
    expect(box("startup-hidden-at-login").disabled).toBe(true); // only with start at login
    fireEvent.click(box("startup-at-login"));
    await waitFor(() => expect(box("startup-at-login").checked).toBe(true));
    expect(mockInvoke).toHaveBeenCalledWith("set_startup_settings", { settings: { ...BASE, start_at_login: true } });
    expect(box("startup-hidden-at-login").disabled).toBe(false);
  });

  it("turning the tray icon off turns off what would hide the window", async () => {
    render(<StartupTraySection />);
    await waitFor(() => expect(box("startup-show-tray").checked).toBe(true));
    fireEvent.click(box("startup-show-tray"));
    await waitFor(() => expect(box("startup-show-tray").checked).toBe(false));
    expect(mockInvoke).toHaveBeenCalledWith("set_startup_settings", {
      settings: { ...BASE, show_tray_icon: false, close_to_tray: false, minimize_to_tray: false },
    });
    expect(box("startup-close-to-tray").disabled).toBe(true);
    expect(box("startup-minimize-to-tray").disabled).toBe(true);
    expect(screen.getByText(/starts minimised in the taskbar/)).toBeInTheDocument();
  });

  it("shows why a change failed", async () => {
    render(<StartupTraySection />);
    await waitFor(() => expect(box("startup-at-login")).toBeInTheDocument());
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "set_startup_settings") throw { message: "Could not change start-at-login: denied", advice: "" };
      return stored;
    });
    fireEvent.click(box("startup-at-login"));
    expect(await screen.findByText(/Could not change start-at-login/)).toBeInTheDocument();
    expect(box("startup-at-login").checked).toBe(false);
  });

  it("says it needs the desktop app in a browser", () => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    render(<StartupTraySection />);
    expect(screen.getByText("Available in the desktop app.")).toBeInTheDocument();
  });
});
