/**
 * T-065: Internationalization (i18n) System
 *
 * Provides locale detection, translation loading, string interpolation,
 * and date/time/number formatting for 8 supported languages.
 *
 * Supported locales: en, es, fr, de, pt, ja, zh, ko
 */

import { create } from "zustand";
import { persist } from "zustand/middleware";
import { scaleBytes } from "@/lib/format-bytes";
import { tauriInvokeSafe } from "@/hooks/use-tauri";
import enTranslations from "../locales/en.json";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** Supported locale codes (BCP 47 language subtag) */
export type Locale = "en" | "es" | "fr" | "de" | "pt" | "ja" | "zh" | "ko";

/** What the user chose in Settings: a language, or follow the system. */
export type LocalePreference = Locale | "system";

/** A flat or nested object of translation strings */
export type TranslationMap = Record<string, string | Record<string, string>>;

/** Parameters for string interpolation: t("hello", { name: "World" }) */
export type InterpolationParams = Record<string, string | number>;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

export const SUPPORTED_LOCALES: readonly Locale[] = [
  "en",
  "es",
  "fr",
  "de",
  "pt",
  "ja",
  "zh",
  "ko",
] as const;

export const LOCALE_NAMES: Record<Locale, string> = {
  en: "English",
  es: "Espanol",
  fr: "Francais",
  de: "Deutsch",
  pt: "Portugues",
  ja: "Japanese",
  zh: "Chinese (Simplified)",
  ko: "Korean",
};

/** Native language names for display in language selector */
export const LOCALE_NATIVE_NAMES: Record<Locale, string> = {
  en: "English",
  es: "Espa\u00f1ol",
  fr: "Fran\u00e7ais",
  de: "Deutsch",
  pt: "Portugu\u00eas",
  ja: "\u65e5\u672c\u8a9e",
  zh: "\u7b80\u4f53\u4e2d\u6587",
  ko: "\ud55c\uad6d\uc5b4",
};

const DEFAULT_LOCALE: Locale = "en";

// ---------------------------------------------------------------------------
// Locale detection
// ---------------------------------------------------------------------------

/**
 * Map a BCP 47 language tag or navigator.language string to our supported Locale.
 * Falls back through increasingly general matches: zh-Hans-CN -> zh-Hans -> zh -> en
 */
function mapToSupportedLocale(tag: string): Locale {
  const normalized = tag.toLowerCase().replace(/_/g, "-");

  // Exact match on primary subtag
  const primary = normalized.split("-")[0] as Locale;
  if (SUPPORTED_LOCALES.includes(primary)) {
    return primary;
  }

  return DEFAULT_LOCALE;
}

/**
 * Detect the user's preferred locale from the browser / OS.
 * Uses navigator.languages (ordered preference list) with fallback.
 */
export function detectLocale(): Locale {
  if (typeof navigator === "undefined") return DEFAULT_LOCALE;

  const languages = navigator.languages ?? [navigator.language];
  for (const lang of languages) {
    const mapped = mapToSupportedLocale(lang);
    if (mapped !== DEFAULT_LOCALE || lang.toLowerCase().startsWith("en")) {
      return mapped;
    }
  }

  return DEFAULT_LOCALE;
}

// ---------------------------------------------------------------------------
// Translation loading
// ---------------------------------------------------------------------------

const translationCache = new Map<Locale, TranslationMap>();

/**
 * Dynamically import a locale JSON file.
 * Uses Vite's dynamic import for code-splitting per locale.
 */
async function loadTranslationFile(locale: Locale): Promise<TranslationMap> {
  if (translationCache.has(locale)) {
    return translationCache.get(locale)!;
  }

  try {
    const module = await import(`../locales/${locale}.json`);
    const translations: TranslationMap = module.default ?? module;
    translationCache.set(locale, translations);
    return translations;
  } catch {
    console.warn(`[i18n] Failed to load locale "${locale}", falling back to "${DEFAULT_LOCALE}"`);
    if (locale !== DEFAULT_LOCALE) {
      return loadTranslationFile(DEFAULT_LOCALE);
    }
    return {};
  }
}

// ---------------------------------------------------------------------------
// String resolution & interpolation
// ---------------------------------------------------------------------------

/**
 * Resolve a dot-notated key from a translation map.
 * Example: resolve("file.actions.copy", translations)
 */
function resolveKey(key: string, map: TranslationMap): string | undefined {
  const parts = key.split(".");
  let current: unknown = map;

  for (const part of parts) {
    if (current == null || typeof current !== "object") return undefined;
    current = (current as Record<string, unknown>)[part];
  }

  return typeof current === "string" ? current : undefined;
}

/**
 * Interpolate {{variable}} placeholders in a translated string.
 */
function interpolate(template: string, params?: InterpolationParams): string {
  if (!params) return template;
  return template.replace(/\{\{(\w+)\}\}/g, (_, key: string) => {
    return params[key] !== undefined ? String(params[key]) : `{{${key}}}`;
  });
}

// ---------------------------------------------------------------------------
// Formatting helpers (locale-aware)
// ---------------------------------------------------------------------------

/**
 * Format a Date according to the current locale.
 */
export function formatDate(
  date: Date | number | string,
  locale: Locale,
  options?: Intl.DateTimeFormatOptions,
): string {
  const d = date instanceof Date ? date : new Date(date);
  const localeTag = localeToTag(locale);
  const defaultOptions: Intl.DateTimeFormatOptions = {
    year: "numeric",
    month: "short",
    day: "numeric",
    ...options,
  };
  return new Intl.DateTimeFormat(localeTag, defaultOptions).format(d);
}

/**
 * Format a Date as a time string according to the current locale.
 */
export function formatTime(
  date: Date | number | string,
  locale: Locale,
  options?: Intl.DateTimeFormatOptions,
): string {
  const d = date instanceof Date ? date : new Date(date);
  const localeTag = localeToTag(locale);
  const defaultOptions: Intl.DateTimeFormatOptions = {
    hour: "2-digit",
    minute: "2-digit",
    ...options,
  };
  return new Intl.DateTimeFormat(localeTag, defaultOptions).format(d);
}

/**
 * Format a number according to the current locale.
 */
export function formatNumber(
  value: number,
  locale: Locale,
  options?: Intl.NumberFormatOptions,
): string {
  const localeTag = localeToTag(locale);
  return new Intl.NumberFormat(localeTag, options).format(value);
}

/**
 * Format file size in a human-readable way.
 */
export function formatFileSize(bytes: number, locale: Locale): string {
  // Units follow Settings → Appearance → Size units (binary or decimal).
  const { value, unit } = scaleBytes(Math.abs(bytes));
  const formatted = formatNumber(
    Math.round(value * 100) / 100,
    locale,
    { minimumFractionDigits: unit !== "B" ? 1 : 0, maximumFractionDigits: 2 },
  );

  return `${formatted} ${unit}`;
}

/**
 * Convert our short locale code to a full BCP 47 tag for Intl APIs.
 */
function localeToTag(locale: Locale): string {
  const map: Record<Locale, string> = {
    en: "en-US",
    es: "es-ES",
    fr: "fr-FR",
    de: "de-DE",
    pt: "pt-BR",
    ja: "ja-JP",
    zh: "zh-CN",
    ko: "ko-KR",
  };
  return map[locale];
}

// ---------------------------------------------------------------------------
// Zustand store
// ---------------------------------------------------------------------------

/** English, bundled: what a key falls back to while a language loads or lacks it. */
const FALLBACK: TranslationMap = enTranslations as TranslationMap;

export function isLocalePreference(v: unknown): v is LocalePreference {
  return v === "system" || (typeof v === "string" && (SUPPORTED_LOCALES as readonly string[]).includes(v));
}

/** The language a preference stands for on this computer. */
export function resolvePreference(pref: LocalePreference): Locale {
  return pref === "system" ? detectLocale() : pref;
}

/** `<html lang>`, and the desktop shell's own texts (tray menu). */
function applyLocale(locale: Locale) {
  if (typeof document !== "undefined") {
    document.documentElement.lang = locale;
    document.documentElement.dir = "ltr"; // All supported locales are LTR
  }
  void tauriInvokeSafe("set_ui_language", { locale }, null);
}

interface I18nState {
  /** The user's choice; `system` follows the computer's language. */
  preference: LocalePreference;
  locale: Locale;
  translations: TranslationMap;
  isLoading: boolean;
  isInitialized: boolean;

  /** Initialize i18n: detect or restore locale, load translations */
  init: () => Promise<void>;

  /** Change locale and load new translations */
  setLocale: (locale: Locale) => Promise<void>;

  /** Choose a language, or `system` to follow the computer's. */
  setPreference: (pref: LocalePreference) => Promise<void>;

  /** Translate a key with optional interpolation params */
  t: (key: string, params?: InterpolationParams) => string;
}

/**
 * v0 stored only `locale`, and English counted as "not chosen": the OS
 * language came back at every start, so choosing English on a German
 * computer did not stick. v1 stores the choice itself. A v0 language other
 * than English was a choice; v0 English is read as "follow the system",
 * which is what v0 did with it.
 */
export function migrateI18nState(persisted: unknown, version: number): { preference: LocalePreference } {
  const p = (persisted && typeof persisted === "object" ? persisted : {}) as { locale?: unknown; preference?: unknown };
  if (version >= 1 && isLocalePreference(p.preference)) return { preference: p.preference };
  if (isLocalePreference(p.locale) && p.locale !== DEFAULT_LOCALE) return { preference: p.locale };
  return { preference: "system" };
}

export const useI18n = create<I18nState>()(
  persist(
    (set, get) => ({
      preference: "system",
      locale: DEFAULT_LOCALE,
      translations: {},
      isLoading: false,
      isInitialized: false,

      init: async () => {
        const state = get();
        if (state.isInitialized) return;

        set({ isLoading: true });
        const locale = resolvePreference(state.preference);
        const translations = await loadTranslationFile(locale);
        applyLocale(locale);
        set({
          locale,
          translations,
          isLoading: false,
          isInitialized: true,
        });
      },

      setLocale: async (locale: Locale) => {
        if (!SUPPORTED_LOCALES.includes(locale)) {
          console.warn(`[i18n] Unsupported locale: ${locale}`);
          return;
        }
        await get().setPreference(locale);
      },

      setPreference: async (pref: LocalePreference) => {
        if (!isLocalePreference(pref)) {
          console.warn(`[i18n] Unsupported locale: ${String(pref)}`);
          return;
        }
        const locale = resolvePreference(pref);
        set({ isLoading: true, preference: pref });
        const translations = await loadTranslationFile(locale);
        applyLocale(locale);
        set({ locale, translations, isLoading: false, isInitialized: true });
      },

      t: (key: string, params?: InterpolationParams): string => {
        const { translations, locale } = get();
        const resolved = resolveKey(key, translations) ?? resolveKey(key, FALLBACK);

        if (resolved === undefined) {
          // In development, warn about missing keys
          if (typeof process !== "undefined" && process.env?.NODE_ENV === "development") {
            console.warn(`[i18n] Missing translation: "${key}" for locale "${locale}"`);
          }
          // Return the key itself as fallback (last segment for readability)
          const segments = key.split(".");
          return segments[segments.length - 1];
        }

        return interpolate(resolved, params);
      },
    }),
    {
      name: "ufop-i18n",
      version: 1,
      migrate: migrateI18nState,
      // Only persist the language choice, not the translations
      partialize: (state) => ({ preference: state.preference }),
    },
  ),
);

// ---------------------------------------------------------------------------
// Convenience exports
// ---------------------------------------------------------------------------

/**
 * Non-hook translation function for use outside React components.
 * Must be called after i18n is initialized.
 */
export function t(key: string, params?: InterpolationParams): string {
  return useI18n.getState().t(key, params);
}

/** Get current locale outside React */
export function getCurrentLocale(): Locale {
  return useI18n.getState().locale;
}

/** `t` for components: re-renders when the language (its translations) changes. */
export function useT(): (key: string, params?: InterpolationParams) => string {
  useI18n((s) => s.translations);
  return useI18n.getState().t;
}
