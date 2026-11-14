import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { migrateI18nState, resolvePreference, SUPPORTED_LOCALES, t, useI18n } from "@/lib/i18n";
import { AppearanceSettings } from "@/components/appearance-settings";
import { BreadcrumbBar } from "@/components/breadcrumb-bar";

import en from "@/locales/en.json";
import es from "@/locales/es.json";
import fr from "@/locales/fr.json";
import de from "@/locales/de.json";
import pt from "@/locales/pt.json";
import ja from "@/locales/ja.json";
import zh from "@/locales/zh.json";
import ko from "@/locales/ko.json";

const FILES: Record<string, unknown> = { en, es, fr, de, pt, ja, zh, ko };

function keys(o: unknown, prefix = ""): string[] {
  return Object.entries(o as Record<string, unknown>).flatMap(([k, v]) =>
    v && typeof v === "object" ? keys(v, `${prefix}${k}.`) : [`${prefix}${k}`],
  );
}

function setLanguages(langs: string[]) {
  Object.defineProperty(navigator, "languages", { value: langs, configurable: true });
}

beforeEach(() => {
  mockInvoke.mockReset().mockResolvedValue(null);
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  setLanguages(["en-US"]);
  useI18n.setState({ preference: "system", locale: "en", translations: {}, isInitialized: false, isLoading: false });
});

describe("language selection", () => {
  it("every language file has exactly the English keys", () => {
    const want = keys(en).sort();
    for (const l of SUPPORTED_LOCALES) expect(keys(FILES[l]).sort(), l).toEqual(want);
  });

  it("falls back to English text, not the key, before a language has loaded", () => {
    expect(t("fileManager.favorites")).toBe("Favorites");
    expect(t("nav.up")).toBe("Go to parent folder");
    expect(t("no.such.key")).toBe("key");
  });

  // Regression: v0 treated English as "not chosen" and detected the OS
  // language again at every start, so English never stuck on a German computer.
  it("keeps English when it was chosen on a computer set to another language", async () => {
    setLanguages(["de-DE"]);
    await useI18n.getState().setPreference("en");
    useI18n.setState({ isInitialized: false, locale: "de" });
    await useI18n.getState().init();
    expect(useI18n.getState().locale).toBe("en");
  });

  it("follows the computer when set to System default", async () => {
    setLanguages(["ja-JP", "en"]);
    await useI18n.getState().init();
    expect(useI18n.getState().locale).toBe("ja");
    expect(resolvePreference("system")).toBe("ja");
    expect(document.documentElement.lang).toBe("ja");
  });

  it("migrates the old stored locale", () => {
    expect(migrateI18nState({ locale: "fr" }, 0)).toEqual({ preference: "fr" });
    expect(migrateI18nState({ locale: "en" }, 0)).toEqual({ preference: "system" });
    expect(migrateI18nState({ locale: "xx" }, 0)).toEqual({ preference: "system" });
    expect(migrateI18nState({ preference: "en" }, 1)).toEqual({ preference: "en" });
    expect(migrateI18nState(null, 0)).toEqual({ preference: "system" });
  });

  it("is chosen in Settings → Appearance, relabels the interface and tells the shell (tray)", async () => {
    render(
      <>
        <AppearanceSettings />
        <BreadcrumbBar path="/home/me" onNavigate={() => {}} onUp={() => {}} canGoUp />
      </>,
    );
    expect(screen.getByTestId("breadcrumb-up")).toHaveAttribute("aria-label", "Go to parent folder");
    await act(async () => {
      fireEvent.change(screen.getByTestId("language-select"), { target: { value: "de" } });
    });
    await waitFor(() => expect(screen.getByTestId("breadcrumb-up")).toHaveAttribute("aria-label", "Zum übergeordneten Ordner"));
    expect(screen.getByText("Sprache")).toBeInTheDocument();
    expect(useI18n.getState().preference).toBe("de");
    expect(document.documentElement.lang).toBe("de");
    expect(mockInvoke).toHaveBeenCalledWith("set_ui_language", { locale: "de" });
  });

  it("offers System default with the detected language's name", () => {
    setLanguages(["fr-FR"]);
    render(<AppearanceSettings />);
    expect(screen.getByRole("option", { name: "System default (Français)" })).toBeInTheDocument();
  });
});
