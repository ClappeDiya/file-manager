/**
 * Accent colour (Settings → Appearance).
 *
 * The app's highlight colour is blue: the design tokens (`--color-primary`,
 * focus ring, selection) and, in many components, Tailwind's `blue-*`
 * utilities. Tailwind 4 reads those utilities from `--color-blue-50…950`, so
 * one stylesheet that redefines that ladder and the primary tokens recolours
 * both. The ladder for another colour keeps Tailwind blue's lightness steps
 * (so text on it stays as readable) and takes the chosen colour's hue and
 * relative chroma, in OKLCH.
 *
 * Blue (the default) adds no stylesheet. High contrast keeps its own colours.
 */

export interface AccentPreset {
  id: string;
  label: string;
  /** The 600 shade, shown on the swatch. */
  hex: string;
}

export const ACCENT_PRESETS: readonly AccentPreset[] = [
  { id: "blue", label: "Blue", hex: "#2563eb" },
  { id: "indigo", label: "Indigo", hex: "#4f46e5" },
  { id: "violet", label: "Violet", hex: "#7c3aed" },
  { id: "pink", label: "Pink", hex: "#db2777" },
  { id: "rose", label: "Rose", hex: "#e11d48" },
  { id: "orange", label: "Orange", hex: "#ea580c" },
  { id: "green", label: "Green", hex: "#16a34a" },
  { id: "teal", label: "Teal", hex: "#0d9488" },
  { id: "graphite", label: "Graphite", hex: "#475569" },
];

/** A preset id or a custom `#rrggbb` colour. */
export type Accent = string;

export const DEFAULT_ACCENT: Accent = "blue";

const HEX = /^#[0-9a-f]{6}$/i;

export function isAccent(v: unknown): v is Accent {
  return typeof v === "string" && (HEX.test(v) || ACCENT_PRESETS.some((p) => p.id === v));
}

/** The colour an accent stands for, as `#rrggbb`. */
export function accentHex(accent: Accent): string {
  if (HEX.test(accent)) return accent.toLowerCase();
  return (ACCENT_PRESETS.find((p) => p.id === accent) ?? ACCENT_PRESETS[0]).hex;
}

export interface Oklch {
  l: number;
  c: number;
  h: number;
}

/** sRGB `#rrggbb` to OKLCH (Björn Ottosson's OKLab). */
export function hexToOklch(hex: string): Oklch {
  const lin = (i: number) => {
    const v = parseInt(hex.slice(i, i + 2), 16) / 255;
    return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  };
  const [r, g, b] = [lin(1), lin(3), lin(5)];
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  const L = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
  const A = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
  const B = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
  const c = Math.hypot(A, B);
  let h = (Math.atan2(B, A) * 180) / Math.PI;
  if (h < 0) h += 360;
  return { l: L, c, h };
}

export const SHADES = [50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 950] as const;
export type Shade = (typeof SHADES)[number];

/** Tailwind 4's blue ladder: lightness and chroma per shade. */
const BLUE: Record<Shade, [number, number]> = {
  50: [0.97, 0.014],
  100: [0.932, 0.032],
  200: [0.882, 0.059],
  300: [0.809, 0.105],
  400: [0.707, 0.165],
  500: [0.623, 0.214],
  600: [0.546, 0.245],
  700: [0.488, 0.243],
  800: [0.424, 0.199],
  900: [0.379, 0.146],
  950: [0.282, 0.091],
};

/** The 50…950 ladder of an accent, as CSS `oklch()` colours. */
export function accentPalette(accent: Accent): Record<Shade, string> {
  const { c, h } = hexToOklch(accentHex(accent));
  // Chroma relative to blue's 600, so a muted colour stays muted.
  const k = Math.min(1.2, c / BLUE[600][1]);
  const out = {} as Record<Shade, string>;
  for (const s of SHADES) {
    const [l, bc] = BLUE[s];
    out[s] = `oklch(${(l * 100).toFixed(1)}% ${(bc * k).toFixed(3)} ${h.toFixed(1)})`;
  }
  return out;
}

/** The stylesheet for an accent; empty for the default blue. */
export function accentCss(accent: Accent): string {
  if (!isAccent(accent) || accentHex(accent) === ACCENT_PRESETS[0].hex) return "";
  const p = accentPalette(accent);
  const ladder = SHADES.map((s) => `--color-blue-${s}: ${p[s]};`).join(" ");
  const light = `--color-primary: ${p[600]}; --color-primary-hover: ${p[700]}; --color-primary-active: ${p[800]}; --color-focus-ring: ${p[600]}; --color-selection-bg: ${p[100]}; --color-selection-text: ${p[800]};`;
  const dark = `--color-primary: ${p[500]}; --color-primary-hover: ${p[400]}; --color-primary-active: ${p[300]}; --color-focus-ring: ${p[500]}; --color-selection-bg: ${p[900]}; --color-selection-text: ${p[300]};`;
  return [
    `:root:not([data-theme="high-contrast"]) { ${ladder} }`,
    `:root[data-theme="light"] { ${light} }`,
    `:root[data-theme="dark"] { ${dark} }`,
    `@media (prefers-color-scheme: dark) { :root:not([data-theme]) { ${dark} } }`,
  ].join("\n");
}

export const ACCENT_STYLE_ID = "ufop-accent";

/** Applies an accent to the page (adds, replaces or removes the stylesheet). */
export function applyAccentToDocument(accent: Accent) {
  if (typeof document === "undefined") return;
  const css = accentCss(accent);
  let el = document.getElementById(ACCENT_STYLE_ID) as HTMLStyleElement | null;
  if (!css) {
    el?.remove();
    return;
  }
  if (!el) {
    el = document.createElement("style");
    el.id = ACCENT_STYLE_ID;
    document.head.appendChild(el);
  }
  el.textContent = css;
}
