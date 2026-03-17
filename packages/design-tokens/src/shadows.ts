/**
 * Shadow tokens for elevation hierarchy.
 * Follows Material-like elevation system adapted for file manager UI.
 */

export const shadows = {
  /** No shadow */
  none: "none",
  /** Subtle shadow for cards and panels on a background */
  xs: "0 1px 2px 0 rgb(0 0 0 / 0.05)",
  /** Default elevation for floating elements */
  sm: "0 1px 3px 0 rgb(0 0 0 / 0.1), 0 1px 2px -1px rgb(0 0 0 / 0.1)",
  /** Medium elevation for dropdowns and popovers */
  md: "0 4px 6px -1px rgb(0 0 0 / 0.1), 0 2px 4px -2px rgb(0 0 0 / 0.1)",
  /** High elevation for modals and dialogs */
  lg: "0 10px 15px -3px rgb(0 0 0 / 0.1), 0 4px 6px -4px rgb(0 0 0 / 0.1)",
  /** Highest elevation for notifications and toasts */
  xl: "0 20px 25px -5px rgb(0 0 0 / 0.1), 0 8px 10px -6px rgb(0 0 0 / 0.1)",
  /** Inner shadow for inset elements (inputs, wells) */
  inner: "inset 0 2px 4px 0 rgb(0 0 0 / 0.05)",
} as const;

/**
 * Dark mode shadows need stronger opacity since background is dark.
 */
export const darkShadows = {
  none: "none",
  xs: "0 1px 2px 0 rgb(0 0 0 / 0.2)",
  sm: "0 1px 3px 0 rgb(0 0 0 / 0.3), 0 1px 2px -1px rgb(0 0 0 / 0.3)",
  md: "0 4px 6px -1px rgb(0 0 0 / 0.3), 0 2px 4px -2px rgb(0 0 0 / 0.3)",
  lg: "0 10px 15px -3px rgb(0 0 0 / 0.3), 0 4px 6px -4px rgb(0 0 0 / 0.3)",
  xl: "0 20px 25px -5px rgb(0 0 0 / 0.3), 0 8px 10px -6px rgb(0 0 0 / 0.3)",
  inner: "inset 0 2px 4px 0 rgb(0 0 0 / 0.2)",
} as const;

export type ShadowKey = keyof typeof shadows;
