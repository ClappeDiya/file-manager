/**
 * Compatibility Badge Component (T-038)
 *
 * Shows a badge/icon on files with active compatibility translations.
 * Displays the number of translations applied and the highest tier.
 */

import React from 'react';

export interface CompatTranslation {
  originalName: string;
  translatedName: string;
  rulesApplied: string[];
  tier: 'auto_quiet' | 'visible_auto' | 'decision_required';
  reversible: boolean;
  profileId: string;
}

interface CompatBadgeProps {
  /** Number of Tier 1 (auto-quiet) translations */
  tier1Count: number;
  /** Number of Tier 2 (visible auto) translations */
  tier2Count: number;
  /** Number of Tier 3 (decision required) translations */
  tier3Count: number;
  /** Callback when badge is clicked to show details */
  onClick?: () => void;
}

/**
 * Small badge showing compatibility translation count on a file/folder.
 * Color indicates the highest tier:
 * - Gray/subtle: Tier 1 only (auto-quiet)
 * - Blue/info: Tier 2 (visible auto)
 * - Orange/warning: Tier 3 (decision required)
 */
export const CompatBadge: React.FC<CompatBadgeProps> = ({
  tier1Count,
  tier2Count,
  tier3Count,
  onClick,
}) => {
  const total = tier1Count + tier2Count + tier3Count;
  if (total === 0) return null;

  const highestTier = tier3Count > 0 ? 3 : tier2Count > 0 ? 2 : 1;

  const badgeStyles: Record<number, React.CSSProperties> = {
    1: {
      backgroundColor: 'var(--compat-tier1-bg, #e2e8f0)',
      color: 'var(--compat-tier1-fg, #475569)',
    },
    2: {
      backgroundColor: 'var(--compat-tier2-bg, #CCFBF1)',
      color: 'var(--compat-tier2-fg, #115E59)',
    },
    3: {
      backgroundColor: 'var(--compat-tier3-bg, #fed7aa)',
      color: 'var(--compat-tier3-fg, #9a3412)',
    },
  };

  const tierIcons: Record<number, string> = {
    1: '\u{2713}',   // check mark
    2: '\u{2139}',   // info
    3: '\u{26A0}',   // warning
  };

  return (
    <button
      onClick={onClick}
      title={`${total} compatibility adjustment${total === 1 ? '' : 's'}`}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: '3px',
        padding: '1px 6px',
        borderRadius: '10px',
        fontSize: '11px',
        fontWeight: 600,
        border: 'none',
        cursor: onClick ? 'pointer' : 'default',
        lineHeight: '16px',
        ...badgeStyles[highestTier],
      }}
      aria-label={`${total} compatibility adjustments, highest tier ${highestTier}`}
    >
      <span>{tierIcons[highestTier]}</span>
      <span>{total}</span>
    </button>
  );
};

export default CompatBadge;
