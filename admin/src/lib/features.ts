/**
 * Which console features have a working backend.
 *
 * A screen whose backend isn't built yet renders <ComingSoon> instead of a
 * form that silently discards input, and its API route answers 501 instead
 * of pretending to store data in process memory. Each flag flips to `true`
 * in the change that ships the feature.
 */
import { NextResponse } from 'next/server';

export const FEATURES = {
  /** SCIM 2.0 user and group provisioning (/api/scim/v2). */
  scim: true,
  /** Server-side CSV user import. */
  csvUserImport: false,
  /** Audit-log forwarding to a SIEM. */
  siemForwarding: false,
  /** Subscription and payment management. */
  billing: false,
  /** Outbound webhook subscriptions with delivery. */
  webhooks: true,
  /** Org-level connector definitions (use Fleet → Credentials meanwhile). */
  connectors: false,
  /** Server-side sync-pair and transfer management. */
  syncApi: false,
  transferApi: false,
} as const;

export type FeatureKey = keyof typeof FEATURES;

export const FEATURE_LABELS: Record<FeatureKey, string> = {
  scim: 'SCIM provisioning',
  csvUserImport: 'CSV user import',
  siemForwarding: 'SIEM forwarding',
  billing: 'Billing',
  webhooks: 'Webhooks',
  connectors: 'Connectors',
  syncApi: 'Sync management API',
  transferApi: 'Transfer management API',
};

export function isEnabled(feature: FeatureKey): boolean {
  return FEATURES[feature];
}

/** 501 for an API whose feature is not built yet. */
export function notAvailable(feature: FeatureKey): NextResponse {
  return NextResponse.json(
    { error: `${FEATURE_LABELS[feature]} is not available yet`, feature },
    { status: 501 },
  );
}
