/** Proxy settings: modes, password handling, test, managed lock. */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { ProxySection, type ProxySettings } from "@/components/proxy-section";

const base: ProxySettings = { mode: "system", url: "", pac_url: "", no_proxy: [], username: "", managed: false };
let stored = base;
beforeEach(() => {
  stored = { ...base };
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  mockInvoke.mockReset().mockImplementation(async (cmd: string, a: { settings: ProxySettings }) => {
    if (cmd === "proxy_get") return { settings: stored, has_password: true, os: { http: "http://sys:8080", https: null, bypass: [], pac_url: null, env: null } };
    if (cmd === "proxy_set") return (stored = a.settings);
    if (cmd === "proxy_test") return { ok: true, status: 204, via: "http://proxy.example:3128/", error: null, millis: 42 };
    return null;
  });
});

describe("ProxySection", () => {
  it("shows the system proxy, then saves a manual one without touching the saved password", async () => {
    render(<ProxySection />);
    expect((await screen.findByTestId("proxy-system")).textContent).toContain("http://sys:8080");
    fireEvent.click(screen.getByLabelText("Manual"));
    fireEvent.change(screen.getByLabelText("Proxy URL"), { target: { value: "http://proxy.example:3128" } });
    fireEvent.change(screen.getByLabelText(/Don't use the proxy/), { target: { value: "localhost, .corp" } });
    expect((screen.getByLabelText("Password") as HTMLInputElement).placeholder).toContain("saved");
    fireEvent.click(screen.getByText("Save"));
    await screen.findByText("Saved");
    const call = mockInvoke.mock.calls.find(([c]) => c === "proxy_set")![1];
    expect(call).toEqual({ settings: { ...base, mode: "manual", url: "http://proxy.example:3128", no_proxy: ["localhost", ".corp"] } });
    expect("password" in call).toBe(false);
  });

  it("sends a new password, and null to clear it", async () => {
    stored = { ...base, mode: "manual", url: "http://p:1" };
    render(<ProxySection />);
    fireEvent.change(await screen.findByLabelText("Password"), { target: { value: "pw" } });
    fireEvent.click(screen.getByText("Save"));
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("proxy_set", expect.objectContaining({ password: "pw" })));
    fireEvent.click(await screen.findByText("Remove saved password"));
    expect(screen.getByText(/will be removed/)).toBeTruthy();
    fireEvent.click(screen.getByText("Save"));
    await waitFor(() => expect(mockInvoke).toHaveBeenLastCalledWith("proxy_set", expect.objectContaining({ password: null })));
  });

  it("tests the connection and locks managed settings", async () => {
    stored = { ...base, managed: true };
    render(<ProxySection />);
    expect(await screen.findByText(/Managed by your organization/)).toBeTruthy();
    expect((screen.getByText("Save") as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByText("Test connection"));
    expect((await screen.findByTestId("proxy-test")).textContent).toBe("Connected in 42 ms · via http://proxy.example:3128/");
  });
});
