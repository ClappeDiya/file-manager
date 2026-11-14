// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
vi.mock("next/navigation", () => ({ usePathname: () => "/dashboard" }));
vi.mock("@/lib/i18n", () => ({ useTranslate: () => (key: string) => key }));
vi.mock("@/lib/stores/auth-store", () => ({
  useAuthStore: (selector: (s: unknown) => unknown) =>
    selector({ checkPermission: () => true }),
}));
vi.mock("@/lib/stores/branding-store", () => ({
  useBrandingStore: (selector: (s: unknown) => unknown) =>
    selector({ branding: null }),
}));
import { Sidebar } from "./sidebar";
import { useAdminStore } from "@/lib/stores/admin-store";
beforeEach(() => {
  vi.stubGlobal("matchMedia", () => ({
    matches: false,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  }));
  vi.spyOn(HTMLElement.prototype, "getClientRects").mockReturnValue([
    {},
  ] as unknown as DOMRectList);
  useAdminStore.persist.setOptions({
    storage: { getItem: () => null, setItem: () => {}, removeItem: () => {} },
  });
  useAdminStore.setState({ sidebarMobileOpen: false, sidebarCollapsed: true });
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});
it("keeps labels visible in the mobile drawer, traps focus, and returns it on Escape", () => {
  function Shell() {
    const open = useAdminStore((s) => s.sidebarMobileOpen);
    return (
      <>
        <button onClick={() => useAdminStore.getState().toggleMobileSidebar()}>
          Open navigation
        </button>
        <Sidebar />
        <p>{String(open)}</p>
      </>
    );
  }
  render(<Shell />);
  const opener = screen.getByRole("button", { name: "Open navigation" });
  opener.focus();
  fireEvent.click(opener);
  const drawer = screen.getByRole("dialog", {
    name: "Administration navigation",
  });
  expect(drawer).toHaveAttribute("aria-modal", "true");
  expect(screen.getByText("nav.dashboard")).toBeInTheDocument();
  expect(drawer.contains(document.activeElement)).toBe(true);
  const last = screen.getByRole("button", { name: "Collapse sidebar" });
  last.focus();
  fireEvent.keyDown(document, { key: "Tab" });
  expect(document.activeElement).not.toBe(last);
  expect(drawer.contains(document.activeElement)).toBe(true);
  fireEvent.keyDown(document, { key: "Escape" });
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(opener).toHaveFocus();
});
