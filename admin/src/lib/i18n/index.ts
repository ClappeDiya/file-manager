'use client';

import { create } from 'zustand';
import { persist, createJSONStorage } from 'zustand/middleware';
import en from '@/locales/en.json';
import de from '@/locales/de.json';
import es from '@/locales/es.json';
import fr from '@/locales/fr.json';
import ja from '@/locales/ja.json';
import ko from '@/locales/ko.json';
import pt from '@/locales/pt.json';
import zh from '@/locales/zh.json';
import { setDisplayLocale } from '@/lib/utils/format';

/**
 * Admin i18n primitive.
 *
 * Same shape as the desktop app's `useI18n` (and the same eight languages):
 *  - Bundles are small JSON trees shipped eagerly; a parity test keeps every
 *    locale's keys equal to English.
 *  - The locale is the signed-in user's choice (My account → Profile, saved
 *    on the account), else the browser's language, else English.
 *  - String lookup uses dotted keys (`nav.dashboard`) to mirror the desktop
 *    catalogue layout; a key missing in a locale falls back to English, an
 *    unknown key to itself (visible in development without breaking layout).
 *  - Dates and numbers follow the locale (`setDisplayLocale`).
 */

export type Locale = 'en' | 'de' | 'es' | 'fr' | 'ja' | 'ko' | 'pt' | 'zh';
export const SUPPORTED_LOCALES: readonly Locale[] = ['en', 'de', 'es', 'fr', 'ja', 'ko', 'pt', 'zh'] as const;
export const LOCALE_NAMES: Record<Locale, string> = {
  en: 'English',
  de: 'Deutsch',
  es: 'Español',
  fr: 'Français',
  ja: '日本語',
  ko: '한국어',
  pt: 'Português',
  zh: '中文',
};

export const isLocale = (v: unknown): v is Locale => typeof v === 'string' && (SUPPORTED_LOCALES as readonly string[]).includes(v);

/** The best supported locale for a browser's language list (e.g. `pt-BR` → `pt`). */
export function matchLocale(languages: readonly string[]): Locale {
  for (const l of languages) {
    const base = l.toLowerCase().split('-')[0];
    if (isLocale(base)) return base;
  }
  return 'en';
}

type TranslationLeaf = string;
type TranslationTree = { [key: string]: TranslationLeaf | TranslationTree };

const CATALOGUE: Record<Locale, TranslationTree> = { en, de, es, fr, ja, ko, pt, zh } as Record<Locale, TranslationTree>;

interface I18nState {
  locale: Locale;
  /** Chosen explicitly (account or this browser) rather than detected. */
  chosen: boolean;
  setLocale: (locale: Locale, chosen?: boolean) => void;
}

const browserLocale = (): Locale => (typeof navigator === 'undefined' ? 'en' : matchLocale(navigator.languages?.length ? navigator.languages : [navigator.language ?? 'en']));

function apply(locale: Locale) {
  setDisplayLocale(locale);
  if (typeof document !== 'undefined') document.documentElement.lang = locale;
}

export const useI18nStore = create<I18nState>()(
  persist(
    (set) => ({
      locale: 'en',
      chosen: false,
      setLocale: (locale, chosen = true) => {
        apply(locale);
        set({ locale, chosen });
      },
    }),
    {
      name: 'ufop-admin-locale',
      storage: createJSONStorage(() => localStorage),
      onRehydrateStorage: () => (state) => {
        // Nothing chosen yet: follow the browser.
        const locale = state?.chosen && isLocale(state.locale) ? state.locale : browserLocale();
        apply(locale);
        if (state && state.locale !== locale) state.locale = locale;
      },
    },
  ),
);

/** Follow the signed-in account's language (null = keep the browser's or this browser's choice). */
export function followAccountLocale(locale: string | null | undefined): void {
  if (isLocale(locale) && useI18nStore.getState().locale !== locale) useI18nStore.getState().setLocale(locale, true);
}

function lookup(tree: TranslationTree, path: string): string | null {
  const segments = path.split('.');
  let node: TranslationTree | TranslationLeaf | undefined = tree;
  for (const segment of segments) {
    if (typeof node !== 'object' || node === null) return null;
    node = (node as TranslationTree)[segment];
    if (node === undefined) return null;
  }
  return typeof node === 'string' ? node : null;
}

function interpolate(template: string, params?: Record<string, string | number>): string {
  if (!params) return template;
  return template.replace(/\{(\w+)\}/g, (_, key) =>
    params[key] !== undefined ? String(params[key]) : `{${key}}`,
  );
}

/**
 * Translate a dotted key. Returns the key itself when unknown so missing
 * strings surface during development without breaking layout.
 *
 *   t('nav.dashboard') -> 'Dashboard'
 *   t('shell.themeLabel', { theme: 'dark' }) -> 'Theme: dark. Activate to switch.'
 */
export function useTranslate(): (key: string, params?: Record<string, string | number>) => string {
  const locale = useI18nStore((s) => s.locale);
  return (key, params) => {
    const tree = CATALOGUE[locale] ?? CATALOGUE.en;
    const direct = lookup(tree, key);
    if (direct !== null) return interpolate(direct, params);
    // Fallback to English if the active locale is missing this key.
    if (locale !== 'en') {
      const englishFallback = lookup(CATALOGUE.en, key);
      if (englishFallback !== null) return interpolate(englishFallback, params);
    }
    return key;
  };
}
