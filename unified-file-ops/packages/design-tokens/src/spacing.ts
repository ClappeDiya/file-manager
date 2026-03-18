/**
 * Spacing scale based on a 4px base unit.
 * Uses a consistent geometric progression for visual harmony.
 */

export const spacing = {
  0: "0px",
  0.5: "2px",
  1: "4px",
  1.5: "6px",
  2: "8px",
  2.5: "10px",
  3: "12px",
  3.5: "14px",
  4: "16px",
  5: "20px",
  6: "24px",
  7: "28px",
  8: "32px",
  9: "36px",
  10: "40px",
  11: "44px",
  12: "48px",
  14: "56px",
  16: "64px",
  20: "80px",
  24: "96px",
  28: "112px",
  32: "128px",
  36: "144px",
  40: "160px",
  44: "176px",
  48: "192px",
  52: "208px",
  56: "224px",
  60: "240px",
  64: "256px",
  72: "288px",
  80: "320px",
  96: "384px",
} as const;

/**
 * Named spacing tokens for common UI patterns.
 */
export const semanticSpacing = {
  /** Inline spacing between icon and text */
  inlineGap: spacing[2],
  /** Padding inside compact components (badges, chips) */
  compactPadding: spacing[1.5],
  /** Standard component padding (buttons, inputs) */
  componentPaddingX: spacing[4],
  componentPaddingY: spacing[2],
  /** Card/panel internal padding */
  cardPadding: spacing[6],
  /** Section spacing between groups */
  sectionGap: spacing[8],
  /** Page-level margins */
  pageMargin: spacing[6],
  /** Sidebar width */
  sidebarWidth: spacing[64],
  /** Sidebar collapsed width */
  sidebarCollapsedWidth: spacing[12],
  /** Toolbar height */
  toolbarHeight: spacing[12],
  /** Status bar height */
  statusBarHeight: spacing[8],
  /** File list row height */
  fileRowHeight: spacing[9],
  /** File list row height compact */
  fileRowHeightCompact: spacing[7],
  /** Tree view indent per level */
  treeIndent: spacing[5],
} as const;

export type SpacingKey = keyof typeof spacing;
export type SemanticSpacingKey = keyof typeof semanticSpacing;
