'use client';

/**
 * The current organization's branding in the console: name and logo in the
 * sidebar, brand color on the primary design tokens.
 */
import { create } from 'zustand';
import { brandTokens } from '@/lib/brand-colors';
import type { EffectiveBranding } from '@/lib/branding';

interface BrandingState {
  branding: Pick<EffectiveBranding, 'displayName' | 'primaryColor' | 'logo' | 'supportEmail' | 'supportUrl'> | null;
  load: () => Promise<void>;
  set: (b: BrandingState['branding']) => void;
}

const TOKEN_NAMES = ['--color-primary', '--color-primary-hover', '--color-primary-active', '--color-primary-foreground'];

export function applyBrandColor(hex: string | null): void {
  if (typeof document === 'undefined') return;
  const root = document.documentElement.style;
  const tokens = brandTokens(hex);
  for (const name of TOKEN_NAMES) {
    if (tokens) root.setProperty(name, tokens[name]);
    else root.removeProperty(name);
  }
}

export const useBrandingStore = create<BrandingState>((set) => ({
  branding: null,
  set: (b) => {
    applyBrandColor(b?.primaryColor ?? null);
    set({ branding: b });
  },
  load: async () => {
    try {
      const r = await fetch('/api/branding', { credentials: 'same-origin' });
      if (!r.ok) return;
      const { branding } = (await r.json()) as { branding: EffectiveBranding };
      applyBrandColor(branding.primaryColor);
      set({ branding });
    } catch {
      // Branding is cosmetic: keep the defaults.
    }
  },
}));
