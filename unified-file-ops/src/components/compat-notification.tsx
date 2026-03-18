/**
 * Compatibility Notification Components (T-038)
 *
 * Implements the three intervention tiers:
 * - Tier 1 (Auto-Quiet): Silent log, badge count only
 * - Tier 2 (Visible Auto): Inline notification, non-blocking, expandable details
 * - Tier 3 (Decision Required): Modal dialog with options
 *
 * Also provides Simple Mode and Advanced Mode message display.
 */

import React, { useState } from 'react';

// ── Types ──

export interface CompatTranslation {
  originalName: string;
  translatedName: string;
  rulesApplied: string[];
  tier: 'auto_quiet' | 'visible_auto' | 'decision_required';
  reversible: boolean;
  profileId: string;
}

export interface CompatBatchResult {
  translations: CompatTranslation[];
  tier1Count: number;
  tier2Count: number;
  tier3Count: number;
  totalModified: number;
  simpleMessage: string;
  advancedDetails: string[];
}

export type Tier3Decision = 'rename' | 'skip' | 'overwrite' | 'cancel';

export interface Tier3Resolution {
  translation: CompatTranslation;
  decision: Tier3Decision;
  applyToAll: boolean;
}

// ── Tier 2: Visible Auto Notification (inline, non-blocking) ──

interface Tier2NotificationProps {
  result: CompatBatchResult;
  /** Whether to show simple or advanced mode */
  advancedMode?: boolean;
  /** Callback to dismiss the notification */
  onDismiss?: () => void;
}

export const Tier2Notification: React.FC<Tier2NotificationProps> = ({
  result,
  advancedMode = false,
  onDismiss,
}) => {
  const [expanded, setExpanded] = useState(false);

  if (result.tier2Count === 0 && result.tier1Count === 0) return null;

  return (
    <div
      role="status"
      aria-live="polite"
      style={{
        padding: '10px 14px',
        backgroundColor: 'var(--compat-notification-bg, #eff6ff)',
        borderLeft: '4px solid var(--compat-notification-border, #0D9488)',
        borderRadius: '0 6px 6px 0',
        marginBottom: '8px',
        fontSize: '13px',
        lineHeight: '1.5',
      }}
    >
      {/* Simple mode message */}
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
        <span>
          {advancedMode
            ? `${result.totalModified} file name${result.totalModified === 1 ? '' : 's'} adjusted for compatibility`
            : result.simpleMessage}
        </span>
        <div style={{ display: 'flex', gap: '8px', alignItems: 'center' }}>
          {result.advancedDetails.length > 0 && (
            <button
              onClick={() => setExpanded(!expanded)}
              style={{
                background: 'none',
                border: 'none',
                color: 'var(--compat-link, #0D9488)',
                cursor: 'pointer',
                fontSize: '12px',
                textDecoration: 'underline',
              }}
              aria-expanded={expanded}
            >
              {expanded ? 'Hide details' : 'Show details'}
            </button>
          )}
          {onDismiss && (
            <button
              onClick={onDismiss}
              style={{
                background: 'none',
                border: 'none',
                cursor: 'pointer',
                fontSize: '16px',
                color: 'var(--compat-dismiss, #94a3b8)',
                lineHeight: 1,
              }}
              aria-label="Dismiss notification"
            >
              x
            </button>
          )}
        </div>
      </div>

      {/* Expanded advanced details */}
      {expanded && (
        <div
          style={{
            marginTop: '8px',
            paddingTop: '8px',
            borderTop: '1px solid var(--compat-divider, #CCFBF1)',
          }}
        >
          {advancedMode ? (
            <ul style={{ margin: 0, paddingLeft: '16px', listStyle: 'none' }}>
              {result.translations
                .filter((t) => t.tier !== 'decision_required')
                .map((t, i) => (
                  <li key={i} style={{ marginBottom: '4px', fontSize: '12px' }}>
                    <span style={{ fontFamily: 'monospace' }}>{t.originalName}</span>
                    {' -> '}
                    <span style={{ fontFamily: 'monospace' }}>{t.translatedName}</span>
                    <br />
                    <span style={{ color: 'var(--compat-rule-text, #6b7280)', fontSize: '11px' }}>
                      {t.rulesApplied.join('; ')}
                    </span>
                  </li>
                ))}
            </ul>
          ) : (
            <ul style={{ margin: 0, paddingLeft: '16px' }}>
              {result.advancedDetails.map((detail, i) => (
                <li key={i} style={{ fontSize: '12px', marginBottom: '2px' }}>
                  {detail}
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
};

// ── Tier 3: Decision Required Modal ──

interface Tier3ModalProps {
  /** The translations that require user decision */
  pendingDecisions: CompatTranslation[];
  /** Callback when user makes a decision */
  onResolve: (resolution: Tier3Resolution) => void;
  /** Callback to cancel the entire operation */
  onCancelAll: () => void;
}

export const Tier3Modal: React.FC<Tier3ModalProps> = ({
  pendingDecisions,
  onResolve,
  onCancelAll,
}) => {
  const [currentIndex, setCurrentIndex] = useState(0);
  const [applyToAll, setApplyToAll] = useState(false);

  if (pendingDecisions.length === 0) return null;

  const current = pendingDecisions[currentIndex];
  const remaining = pendingDecisions.length - currentIndex;

  const handleDecision = (decision: Tier3Decision) => {
    if (applyToAll) {
      // Apply to all remaining
      for (let i = currentIndex; i < pendingDecisions.length; i++) {
        onResolve({
          translation: pendingDecisions[i],
          decision,
          applyToAll: true,
        });
      }
    } else {
      onResolve({
        translation: current,
        decision,
        applyToAll: false,
      });
      if (currentIndex < pendingDecisions.length - 1) {
        setCurrentIndex(currentIndex + 1);
      }
    }
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-labelledby="compat-modal-title"
      style={{
        position: 'fixed',
        top: 0,
        left: 0,
        right: 0,
        bottom: 0,
        backgroundColor: 'rgba(0, 0, 0, 0.5)',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        zIndex: 1000,
      }}
    >
      <div
        style={{
          backgroundColor: 'var(--compat-modal-bg, #ffffff)',
          borderRadius: '8px',
          padding: '24px',
          maxWidth: '520px',
          width: '90%',
          boxShadow: '0 20px 60px rgba(0,0,0,0.3)',
        }}
      >
        {/* Header */}
        <h2
          id="compat-modal-title"
          style={{
            margin: '0 0 16px',
            fontSize: '16px',
            fontWeight: 600,
          }}
        >
          Compatibility Issue ({remaining} remaining)
        </h2>

        {/* Issue details */}
        <div
          style={{
            padding: '12px',
            backgroundColor: 'var(--compat-issue-bg, #fef3c7)',
            borderRadius: '6px',
            marginBottom: '16px',
          }}
        >
          <div style={{ marginBottom: '8px' }}>
            <strong>File:</strong>{' '}
            <span style={{ fontFamily: 'monospace' }}>{current.originalName}</span>
          </div>
          <div style={{ marginBottom: '8px' }}>
            <strong>Issue:</strong> {current.rulesApplied.join('; ')}
          </div>
          {current.translatedName !== current.originalName && (
            <div>
              <strong>Suggested name:</strong>{' '}
              <span style={{ fontFamily: 'monospace' }}>{current.translatedName}</span>
            </div>
          )}
        </div>

        {/* Apply to all checkbox */}
        {remaining > 1 && (
          <label
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: '8px',
              marginBottom: '16px',
              fontSize: '13px',
              cursor: 'pointer',
            }}
          >
            <input
              type="checkbox"
              checked={applyToAll}
              onChange={(e) => setApplyToAll(e.target.checked)}
            />
            Apply to all {remaining} remaining items
          </label>
        )}

        {/* Action buttons */}
        <div style={{ display: 'flex', gap: '8px', justifyContent: 'flex-end' }}>
          <button
            onClick={() => handleDecision('skip')}
            style={{
              padding: '8px 16px',
              border: '1px solid var(--compat-btn-border, #d1d5db)',
              borderRadius: '6px',
              backgroundColor: 'var(--compat-btn-secondary-bg, #f9fafb)',
              cursor: 'pointer',
              fontSize: '13px',
            }}
          >
            Skip
          </button>
          <button
            onClick={() => handleDecision('overwrite')}
            style={{
              padding: '8px 16px',
              border: '1px solid var(--compat-btn-border, #d1d5db)',
              borderRadius: '6px',
              backgroundColor: 'var(--compat-btn-secondary-bg, #f9fafb)',
              cursor: 'pointer',
              fontSize: '13px',
            }}
          >
            Keep original
          </button>
          <button
            onClick={() => handleDecision('rename')}
            style={{
              padding: '8px 16px',
              border: 'none',
              borderRadius: '6px',
              backgroundColor: 'var(--compat-btn-primary-bg, #0D9488)',
              color: 'var(--compat-btn-primary-fg, #ffffff)',
              cursor: 'pointer',
              fontSize: '13px',
              fontWeight: 600,
            }}
          >
            Rename
          </button>
          <button
            onClick={onCancelAll}
            style={{
              padding: '8px 16px',
              border: '1px solid var(--compat-btn-danger-border, #fca5a5)',
              borderRadius: '6px',
              backgroundColor: 'var(--compat-btn-danger-bg, #fef2f2)',
              color: 'var(--compat-btn-danger-fg, #991b1b)',
              cursor: 'pointer',
              fontSize: '13px',
            }}
          >
            Cancel all
          </button>
        </div>
      </div>
    </div>
  );
};

// ── Combined CompatNotificationPanel ──

interface CompatNotificationPanelProps {
  result: CompatBatchResult;
  advancedMode?: boolean;
  onDismiss?: () => void;
  onTier3Resolve?: (resolution: Tier3Resolution) => void;
  onCancelAll?: () => void;
}

/**
 * Combined notification panel that renders the appropriate UI
 * based on intervention tiers present in the batch result.
 */
export const CompatNotificationPanel: React.FC<CompatNotificationPanelProps> = ({
  result,
  advancedMode = false,
  onDismiss,
  onTier3Resolve,
  onCancelAll,
}) => {
  const tier3Pending = result.translations.filter(
    (t) => t.tier === 'decision_required'
  );

  return (
    <>
      {/* Tier 1 + 2: Inline notification */}
      {(result.tier1Count > 0 || result.tier2Count > 0) && (
        <Tier2Notification
          result={result}
          advancedMode={advancedMode}
          onDismiss={onDismiss}
        />
      )}

      {/* Tier 3: Modal for decision-required items */}
      {tier3Pending.length > 0 && onTier3Resolve && onCancelAll && (
        <Tier3Modal
          pendingDecisions={tier3Pending}
          onResolve={onTier3Resolve}
          onCancelAll={onCancelAll}
        />
      )}
    </>
  );
};

export default CompatNotificationPanel;
