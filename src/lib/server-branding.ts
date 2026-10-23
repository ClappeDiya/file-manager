/**
 * ufop-server web UI branding (K124). The server injects the public values
 * into index.html (`window.__UFOP_BRANDING__`, already applied to the
 * title and the `--ufop-accent` CSS variable before the first paint) and
 * serves them at `GET /api/v1/branding` (no sign-in). Mirrors Rust
 * `server::branding::PublicBranding`.
 */
import { useEffect, useState } from "react";

export interface PublicBranding {
  title_prefix: string;
  product_name: string;
  accent_color: string | null;
  /** `/branding/logo?v=…` when a logo is set. */
  logo_url: string | null;
  /** `/branding/custom.css?v=…` when custom CSS is set. */
  css_url: string | null;
  revision: string;
}

declare global {
  interface Window {
    __UFOP_BRANDING__?: PublicBranding;
  }
}

export const DEFAULT_PRODUCT_NAME = "UFOP backup server";

export function initialBranding(): PublicBranding | null {
  return typeof window !== "undefined" ? (window.__UFOP_BRANDING__ ?? null) : null;
}

export async function fetchBranding(): Promise<PublicBranding | null> {
  try {
    const r = await fetch(`${window.__UFOP_SERVER__?.api ?? "/api/v1"}/branding`);
    return r.ok ? ((await r.json()) as PublicBranding) : null;
  } catch {
    return null;
  }
}

/** Product name to show (the branding's, else the default). */
export function productName(b: PublicBranding | null): string {
  return b?.product_name.trim() || DEFAULT_PRODUCT_NAME;
}

/** Page title for `page` ("Backups") under the branding. */
export function brandedTitle(b: PublicBranding | null, page?: string): string {
  const name = productName(b);
  return `${b?.title_prefix ?? ""}${page ? `${page} · ${name}` : name}`;
}

/** Apply title, accent variable, stylesheet and favicon to the document. */
export function applyBranding(b: PublicBranding | null, page?: string): void {
  if (typeof document === "undefined") return;
  document.title = brandedTitle(b, page);
  const root = document.documentElement;
  if (b?.accent_color) root.style.setProperty("--ufop-accent", b.accent_color);
  else root.style.removeProperty("--ufop-accent");
  const css = document.querySelector<HTMLLinkElement>('link[data-ufop-branding="css"]') ?? document.querySelector<HTMLLinkElement>('link[href^="/branding/custom.css"]');
  if (b?.css_url) {
    const link = css ?? Object.assign(document.createElement("link"), { rel: "stylesheet" });
    link.dataset.ufopBranding = "css";
    if (link.getAttribute("href") !== b.css_url) link.setAttribute("href", b.css_url);
    if (!link.isConnected) document.head.appendChild(link);
  } else {
    css?.remove();
  }
  if (b?.logo_url) {
    const icon = document.querySelector<HTMLLinkElement>('link[rel="icon"]') ?? Object.assign(document.createElement("link"), { rel: "icon" });
    icon.setAttribute("href", b.logo_url);
    if (!icon.isConnected) document.head.appendChild(icon);
  }
}

/** Branding for the server web UI: the injected values, refreshed from the API. */
export function useServerBranding(page?: string): PublicBranding | null {
  const [b, setB] = useState<PublicBranding | null>(initialBranding);
  useEffect(() => {
    let live = true;
    void fetchBranding().then((x) => {
      if (live && x) setB(x);
    });
    return () => {
      live = false;
    };
  }, []);
  useEffect(() => applyBranding(b, page), [b, page]);
  return b;
}

/** Inline style for an accent-colored control (falls back to the theme's blue class). */
export function accentStyle(b: PublicBranding | null): { backgroundColor: string } | undefined {
  return b?.accent_color ? { backgroundColor: "var(--ufop-accent)" } : undefined;
}
