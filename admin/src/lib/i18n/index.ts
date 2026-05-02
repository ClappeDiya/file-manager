'use client';

import { create } from 'zustand';
import { persist, createJSONStorage } from 'zustand/middleware';
import en from '@/locales/en.json';

/**
 * Admin i18n primitive.
 *
 * Same shape as the desktop app's `useI18n` so future shared extraction is
 * straightforward, but Next-friendly:
 *  - English bundle ships eagerly (the only locale in this PR).
 *  - Additional locales would be added via dynamic `import()` keyed off the
 *    `Locale` union; that work is intentionally scoped out of this change.
 *  - String lookup uses dotted keys (`nav.dashboard`) to mirror the desktop
 *    catalogue layout.
 *  - Unknown keys fall back to the key itself so missing strings are visible
 *    in development without breaking the UI.
 */

export type Locale = 'en';
export const SUPPORTED_LOCALES: readonly Locale[] = ['en'] as const;
export const LOCALE_NAMES: Record<Locale, string> = {
  en: 'English',
};

type TranslationLeaf = string;
type TranslationTree = { [key: string]: TranslationLeaf | TranslationTree };

const CATALOGUE: Record<Locale, TranslationTree> = {
  en: en as TranslationTree,
};

interface I18nState {
  locale: Locale;
  setLocale: (locale: Locale) => void;
}

export const useI18nStore = create<I18nState>()(
  persist(
    (set) => ({
      locale: 'en',
      setLocale: (locale) => set({ locale }),
    }),
    { name: 'ufop-admin-locale', storage: createJSONStorage(() => localStorage) },
  ),
);

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
