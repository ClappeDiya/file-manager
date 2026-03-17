/**
 * @ufop/design-tokens
 *
 * Central design token package for the Unified File Operations Platform.
 * Provides colors, spacing, typography, shadows, radii, and animations
 * for light, dark, and high-contrast themes.
 */

export {
  lightColors,
  darkColors,
  highContrastColors,
  type ColorTokens,
  type ColorKey,
} from "./colors";

export {
  spacing,
  semanticSpacing,
  type SpacingKey,
  type SemanticSpacingKey,
} from "./spacing";

export {
  fontFamilies,
  fontSizes,
  lineHeights,
  fontWeights,
  letterSpacings,
  textStyles,
  type FontSizeKey,
  type TextStyleKey,
} from "./typography";

export { shadows, darkShadows, type ShadowKey } from "./shadows";

export { radii, type RadiusKey } from "./radii";

export {
  durations,
  easings,
  transitions,
  type DurationKey,
  type EasingKey,
} from "./animations";
