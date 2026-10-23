/**
 * ufop-server web UI branding (K124): the sign-in screen and document title
 * use the product name, title prefix, logo and accent color from
 * `window.__UFOP_BRANDING__` / `GET /api/v1/branding`.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import { applyBranding, brandedTitle, productName, type PublicBranding } from "@/lib/server-branding";
import ServerApp from "@/components/server/server-app";

const ACME: PublicBranding = {
  title_prefix: "[ACME] ",
  product_name: "ACME Backup",
  accent_color: "#1e66f5",
  logo_url: "/branding/logo?v=1",
  css_url: "/branding/custom.css?v=1",
  revision: "1",
};

const w = window as unknown as Record<string, unknown>;

beforeEach(() => {
  w.__UFOP_SERVER__ = { api: "/api/v1" };
  delete w.__UFOP_BRANDING__;
  document.title = "";
  document.documentElement.style.removeProperty("--ufop-accent");
  document.head.querySelectorAll("link").forEach((l) => l.remove());
});

afterEach(() => {
  vi.unstubAllGlobals();
  delete w.__UFOP_SERVER__;
});

describe("branding helpers", () => {
  it("builds titles with the prefix and product name", () => {
    expect(brandedTitle(ACME, "Backups")).toBe("[ACME] Backups · ACME Backup");
    expect(brandedTitle(null)).toBe("UFOP backup server");
    expect(productName({ ...ACME, product_name: "  " })).toBe("UFOP backup server");
  });

  it("applies title, accent, stylesheet and favicon, and removes them again", () => {
    applyBranding(ACME, "Sign in");
    expect(document.title).toBe("[ACME] Sign in · ACME Backup");
    expect(document.documentElement.style.getPropertyValue("--ufop-accent")).toBe("#1e66f5");
    expect(document.head.querySelector('link[data-ufop-branding="css"]')?.getAttribute("href")).toBe("/branding/custom.css?v=1");
    expect(document.head.querySelector('link[rel="icon"]')?.getAttribute("href")).toBe("/branding/logo?v=1");
    applyBranding(null);
    expect(document.documentElement.style.getPropertyValue("--ufop-accent")).toBe("");
    expect(document.head.querySelector('link[data-ufop-branding="css"]')).toBeNull();
  });
});

describe("server web UI", () => {
  it("brands the sign-in screen from the API", async () => {
    const fetchMock = vi.fn(async (url: string) => {
      if (url.endsWith("/branding")) return new Response(JSON.stringify(ACME), { status: 200 });
      return new Response(JSON.stringify({ message: "no session" }), { status: 401 });
    });
    vi.stubGlobal("fetch", fetchMock);
    render(<ServerApp />);
    expect(await screen.findByTestId("server-product-name")).toHaveTextContent("ACME Backup");
    expect(screen.getByTestId("server-brand-logo")).toHaveAttribute("src", "/branding/logo?v=1");
    expect(screen.getByRole("button", { name: /sign in/i })).toHaveStyle({ backgroundColor: "var(--ufop-accent)" });
    await waitFor(() => expect(document.title).toBe("[ACME] Sign in · ACME Backup"));
  });

  it("uses the injected values before the API answers and the default without branding", async () => {
    w.__UFOP_BRANDING__ = { ...ACME, product_name: "Injected" };
    vi.stubGlobal("fetch", vi.fn(async () => new Response("{}", { status: 500 })));
    render(<ServerApp />);
    expect(await screen.findByTestId("server-product-name")).toHaveTextContent("Injected");
  });

  it("shows the default name without branding", async () => {
    vi.stubGlobal("fetch", vi.fn(async () => new Response("{}", { status: 404 })));
    render(<ServerApp />);
    expect(await screen.findByTestId("server-product-name")).toHaveTextContent("UFOP backup server");
    expect(screen.queryByTestId("server-brand-logo")).toBeNull();
  });
});
