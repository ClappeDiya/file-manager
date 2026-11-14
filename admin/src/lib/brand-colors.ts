/** Brand color variants for the console's design tokens (`--color-primary*`). */

const clamp = (n: number) => Math.max(0, Math.min(255, Math.round(n)));

/** `hex` darkened (amount < 0) or lightened (amount > 0) by a fraction. */
export function shade(hex: string, amount: number): string {
  const n = parseInt(hex.slice(1), 16);
  const ch = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map((c) => clamp(amount < 0 ? c * (1 + amount) : c + (255 - c) * amount));
  return `#${ch.map((c) => c.toString(16).padStart(2, '0')).join('')}`;
}

/** Black or white, whichever reads better on `hex`. */
export function foregroundOn(hex: string): string {
  const n = parseInt(hex.slice(1), 16);
  const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  return 0.299 * r + 0.587 * g + 0.114 * b > 160 ? '#111111' : '#ffffff';
}

/** The token values for a brand color (null = the design tokens' own). */
export function brandTokens(hex: string | null): Record<string, string> | null {
  if (!hex || !/^#[0-9a-f]{6}$/i.test(hex)) return null;
  return {
    '--color-primary': hex,
    '--color-primary-hover': shade(hex, -0.12),
    '--color-primary-active': shade(hex, -0.24),
    '--color-primary-foreground': foregroundOn(hex),
  };
}
