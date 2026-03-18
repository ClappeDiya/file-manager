/**
 * Animation tokens.
 * All animations respect prefers-reduced-motion via CSS.
 */

export const durations = {
  instant: "0ms",
  fast: "100ms",
  normal: "200ms",
  slow: "300ms",
  slower: "500ms",
} as const;

export const easings = {
  /** Standard easing for most transitions */
  default: "cubic-bezier(0.4, 0, 0.2, 1)",
  /** Ease in for elements entering */
  in: "cubic-bezier(0.4, 0, 1, 1)",
  /** Ease out for elements leaving */
  out: "cubic-bezier(0, 0, 0.2, 1)",
  /** Spring-like easing for playful interactions */
  spring: "cubic-bezier(0.34, 1.56, 0.64, 1)",
} as const;

export const transitions = {
  /** For color changes (hover states, theme switching) */
  colors: `color ${durations.fast} ${easings.default}, background-color ${durations.fast} ${easings.default}, border-color ${durations.fast} ${easings.default}`,
  /** For opacity changes (fade in/out) */
  opacity: `opacity ${durations.normal} ${easings.default}`,
  /** For transform changes (scale, translate) */
  transform: `transform ${durations.normal} ${easings.default}`,
  /** For shadow changes (elevation) */
  shadow: `box-shadow ${durations.normal} ${easings.default}`,
  /** Combined common transition */
  all: `all ${durations.normal} ${easings.default}`,
} as const;

export type DurationKey = keyof typeof durations;
export type EasingKey = keyof typeof easings;
