/**
 * AI governance settings, one row per organization. Every change is also
 * written to the hash-chained audit log (`ai.governance.*` events), which
 * is where the "AI audit" view reads from.
 */
import { db, now, parseJson, type Row } from '../index';

export type DataResidency = 'local' | 'cloud' | 'hybrid';

export const AI_FEATURES = ['classification', 'content_analysis', 'naming', 'anomaly', 'auto_suggestions', 'natural_language_commands'] as const;
export type AiFeature = (typeof AI_FEATURES)[number];

export const AI_ACTIONS = ['organize', 'rename', 'tag', 'move', 'copy', 'delete', 'sync', 'transfer'] as const;

export interface AiGovernanceSettings {
  aiEnabled: boolean;
  features: Record<AiFeature, boolean>;
  dataResidency: DataResidency;
  /** Paths AI features must never read. */
  excludedPaths: string[];
  requireConfirmationForDestructive: boolean;
  maxFilesPerAction: number;
  allowedActions: string[];
  auditRetentionDays: number;
}

export const DEFAULT_AI_GOVERNANCE: AiGovernanceSettings = {
  aiEnabled: true,
  features: {
    classification: true,
    content_analysis: false,
    naming: true,
    anomaly: true,
    auto_suggestions: true,
    natural_language_commands: true,
  },
  dataResidency: 'local',
  excludedPaths: [],
  requireConfirmationForDestructive: true,
  maxFilesPerAction: 100,
  allowedActions: ['organize', 'rename', 'tag', 'move', 'copy'],
  auditRetentionDays: 90,
};

export class GovernanceError extends Error {}

export function getAiGovernance(orgId: string): AiGovernanceSettings & { updatedAt: string | null; updatedBy: string } {
  const r = db().prepare('SELECT * FROM ai_governance WHERE org_id = ?').get(orgId) as Row | undefined;
  const stored = r ? parseJson<Partial<AiGovernanceSettings>>(r.settings, {}) : {};
  return {
    ...DEFAULT_AI_GOVERNANCE,
    ...stored,
    features: { ...DEFAULT_AI_GOVERNANCE.features, ...(stored.features ?? {}) },
    updatedAt: r ? (r.updated_at as string) : null,
    updatedBy: r ? (r.updated_by as string) : '',
  };
}

/** Validate a partial update against the current settings. */
export function validateAiGovernance(patch: Record<string, unknown>, current: AiGovernanceSettings): AiGovernanceSettings {
  const next: AiGovernanceSettings = { ...current, features: { ...current.features } };
  const bool = (k: string) => {
    if (typeof patch[k] !== 'boolean') throw new GovernanceError(`${k} must be true or false`);
    return patch[k] as boolean;
  };
  if ('aiEnabled' in patch) next.aiEnabled = bool('aiEnabled');
  if ('requireConfirmationForDestructive' in patch) next.requireConfirmationForDestructive = bool('requireConfirmationForDestructive');
  if ('features' in patch) {
    const f = patch.features;
    if (!f || typeof f !== 'object' || Array.isArray(f)) throw new GovernanceError('features must be an object');
    for (const [k, v] of Object.entries(f)) {
      if (!(AI_FEATURES as readonly string[]).includes(k)) throw new GovernanceError(`Unknown AI feature: ${k}`);
      if (typeof v !== 'boolean') throw new GovernanceError(`features.${k} must be true or false`);
      next.features[k as AiFeature] = v;
    }
  }
  if ('dataResidency' in patch) {
    if (!['local', 'cloud', 'hybrid'].includes(patch.dataResidency as string)) throw new GovernanceError('dataResidency must be local, cloud or hybrid');
    next.dataResidency = patch.dataResidency as DataResidency;
  }
  if ('excludedPaths' in patch) {
    const p = patch.excludedPaths;
    if (!Array.isArray(p) || p.some((x) => typeof x !== 'string')) throw new GovernanceError('excludedPaths must be a list of paths');
    next.excludedPaths = [...new Set(p.map((x: string) => x.trim()).filter(Boolean))].slice(0, 500);
  }
  if ('maxFilesPerAction' in patch) {
    const n = patch.maxFilesPerAction;
    if (!Number.isInteger(n) || (n as number) < 1 || (n as number) > 100_000) throw new GovernanceError('maxFilesPerAction must be between 1 and 100000');
    next.maxFilesPerAction = n as number;
  }
  if ('auditRetentionDays' in patch) {
    const n = patch.auditRetentionDays;
    if (!Number.isInteger(n) || (n as number) < 1 || (n as number) > 3650) throw new GovernanceError('auditRetentionDays must be between 1 and 3650');
    next.auditRetentionDays = n as number;
  }
  if ('allowedActions' in patch) {
    const a = patch.allowedActions;
    if (!Array.isArray(a) || a.some((x) => !(AI_ACTIONS as readonly string[]).includes(x as string))) {
      throw new GovernanceError(`allowedActions may contain: ${AI_ACTIONS.join(', ')}`);
    }
    next.allowedActions = [...new Set(a as string[])];
  }
  return next;
}

export function saveAiGovernance(orgId: string, settings: AiGovernanceSettings, updatedBy: string): void {
  db()
    .prepare(
      `INSERT INTO ai_governance (org_id, settings, updated_by, updated_at) VALUES (?, ?, ?, ?)
       ON CONFLICT(org_id) DO UPDATE SET settings = excluded.settings, updated_by = excluded.updated_by, updated_at = excluded.updated_at`,
    )
    .run(orgId, JSON.stringify(settings), updatedBy, now());
}
