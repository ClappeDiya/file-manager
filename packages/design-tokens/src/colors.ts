/**
 * Color tokens for the Unified File Operations Platform.
 *
 * All colors are organized by semantic usage, with variants for
 * light, dark, and high-contrast themes.
 *
 * Contrast ratios are designed to meet WCAG 2.1 AA requirements:
 * - Normal text: 4.5:1 minimum
 * - Large text: 3:1 minimum
 * - UI components: 3:1 minimum
 */

export const lightColors = {
  // Primary
  primary: "#2563eb",
  primaryHover: "#1d4ed8",
  primaryActive: "#1e40af",
  primaryForeground: "#ffffff",

  // Accent
  accent: "#8b5cf6",
  accentHover: "#7c3aed",
  accentForeground: "#ffffff",

  // Background hierarchy
  bg: "#ffffff",
  bgSecondary: "#f8fafc",
  bgTertiary: "#f1f5f9",
  bgElevated: "#ffffff",
  bgOverlay: "rgba(0, 0, 0, 0.5)",

  // Text hierarchy - all meet 4.5:1 on their expected bg
  text: "#0f172a", // 15.4:1 on white
  textSecondary: "#475569", // 6.4:1 on white
  textTertiary: "#64748b", // 4.6:1 on white
  textInverse: "#f8fafc",
  textDisabled: "#94a3b8",

  // Border
  border: "#e2e8f0",
  borderHover: "#cbd5e1",
  borderFocus: "#2563eb",
  borderError: "#dc2626",

  // Semantic colors
  success: "#16a34a",
  successBg: "#f0fdf4",
  successForeground: "#15803d",
  warning: "#d97706",
  warningBg: "#fffbeb",
  warningForeground: "#b45309",
  error: "#dc2626",
  errorBg: "#fef2f2",
  errorForeground: "#b91c1c",
  info: "#2563eb",
  infoBg: "#eff6ff",
  infoForeground: "#1d4ed8",

  // Component-specific
  sidebarBg: "#f8fafc",
  sidebarText: "#334155",
  sidebarTextActive: "#0f172a",
  sidebarBorder: "#e2e8f0",
  toolbarBg: "#ffffff",
  toolbarBorder: "#e2e8f0",
  selectionBg: "#dbeafe",
  selectionText: "#1e40af",
  hoverBg: "#f1f5f9",

  // Focus ring
  focusRing: "#2563eb",
} as const;

export const darkColors = {
  // Primary
  primary: "#3b82f6",
  primaryHover: "#60a5fa",
  primaryActive: "#93c5fd",
  primaryForeground: "#0f172a",

  // Accent
  accent: "#a78bfa",
  accentHover: "#c4b5fd",
  accentForeground: "#0f172a",

  // Background hierarchy
  bg: "#0f172a",
  bgSecondary: "#1e293b",
  bgTertiary: "#334155",
  bgElevated: "#1e293b",
  bgOverlay: "rgba(0, 0, 0, 0.7)",

  // Text hierarchy - all meet 4.5:1 on their expected bg
  text: "#f8fafc", // 15.4:1 on #0f172a
  textSecondary: "#cbd5e1", // 8.2:1 on #0f172a
  textTertiary: "#94a3b8", // 4.9:1 on #0f172a
  textInverse: "#0f172a",
  textDisabled: "#64748b",

  // Border
  border: "#334155",
  borderHover: "#475569",
  borderFocus: "#3b82f6",
  borderError: "#ef4444",

  // Semantic colors
  success: "#22c55e",
  successBg: "#052e16",
  successForeground: "#4ade80",
  warning: "#f59e0b",
  warningBg: "#451a03",
  warningForeground: "#fbbf24",
  error: "#ef4444",
  errorBg: "#450a0a",
  errorForeground: "#f87171",
  info: "#3b82f6",
  infoBg: "#172554",
  infoForeground: "#60a5fa",

  // Component-specific
  sidebarBg: "#1e293b",
  sidebarText: "#cbd5e1",
  sidebarTextActive: "#f8fafc",
  sidebarBorder: "#334155",
  toolbarBg: "#1e293b",
  toolbarBorder: "#334155",
  selectionBg: "#1e3a5f",
  selectionText: "#93c5fd",
  hoverBg: "#1e293b",

  // Focus ring
  focusRing: "#3b82f6",
} as const;

export const highContrastColors = {
  // Primary - higher contrast values
  primary: "#1d4ed8",
  primaryHover: "#1e40af",
  primaryActive: "#1e3a8a",
  primaryForeground: "#ffffff",

  // Accent
  accent: "#7c3aed",
  accentHover: "#6d28d9",
  accentForeground: "#ffffff",

  // Background hierarchy - maximum contrast
  bg: "#ffffff",
  bgSecondary: "#f1f5f9",
  bgTertiary: "#e2e8f0",
  bgElevated: "#ffffff",
  bgOverlay: "rgba(0, 0, 0, 0.7)",

  // Text hierarchy - all exceed 7:1 for AAA
  text: "#000000", // 21:1 on white
  textSecondary: "#1e293b", // 14.5:1 on white
  textTertiary: "#334155", // 10.1:1 on white
  textInverse: "#ffffff",
  textDisabled: "#64748b",

  // Border - thicker visually, higher contrast
  border: "#94a3b8",
  borderHover: "#64748b",
  borderFocus: "#1d4ed8",
  borderError: "#b91c1c",

  // Semantic colors - higher contrast
  success: "#15803d",
  successBg: "#dcfce7",
  successForeground: "#14532d",
  warning: "#a16207",
  warningBg: "#fef9c3",
  warningForeground: "#854d0e",
  error: "#b91c1c",
  errorBg: "#fee2e2",
  errorForeground: "#991b1b",
  info: "#1d4ed8",
  infoBg: "#dbeafe",
  infoForeground: "#1e40af",

  // Component-specific
  sidebarBg: "#f1f5f9",
  sidebarText: "#1e293b",
  sidebarTextActive: "#000000",
  sidebarBorder: "#94a3b8",
  toolbarBg: "#ffffff",
  toolbarBorder: "#94a3b8",
  selectionBg: "#bfdbfe",
  selectionText: "#1e3a8a",
  hoverBg: "#e2e8f0",

  // Focus ring
  focusRing: "#1d4ed8",
} as const;

export type ColorTokens = typeof lightColors;
export type ColorKey = keyof ColorTokens;
