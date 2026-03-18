/**
 * Policy engine utilities (T-050).
 * Resolves effective policies considering precedence: user > role > org.
 */

import {
  type PolicyRule,
  type PolicyDomain,
  type PolicyAssignmentTarget,
  POLICY_PRECEDENCE,
} from '../types/policies';

interface PolicyResolutionContext {
  userId: string;
  userRole: string;
  organizationId: string;
  connectionType?: string;
}

/**
 * Resolve the effective policy for a given domain, considering precedence.
 * Precedence: user (30) > role (20) > connection_type (15) > org (10)
 */
export function resolveEffectivePolicy(
  domain: PolicyDomain,
  policies: PolicyRule[],
  context: PolicyResolutionContext
): PolicyRule | null {
  // Filter to active policies for this domain
  const domainPolicies = policies.filter(
    p => p.domain === domain && p.isActive && p.enforcementMode !== 'disabled'
  );

  if (domainPolicies.length === 0) return null;

  // Find matching policies with precedence scoring
  const candidates: { policy: PolicyRule; precedence: number }[] = [];

  for (const policy of domainPolicies) {
    let matches = false;

    switch (policy.assignmentTarget) {
      case 'user':
        matches = policy.assignmentValue === context.userId;
        break;
      case 'role':
        matches = policy.assignmentValue === context.userRole;
        break;
      case 'org':
        matches = policy.assignmentValue === context.organizationId;
        break;
      case 'connection_type':
        matches = policy.assignmentValue === context.connectionType;
        break;
    }

    if (matches) {
      candidates.push({
        policy,
        precedence: POLICY_PRECEDENCE[policy.assignmentTarget],
      });
    }
  }

  if (candidates.length === 0) return null;

  // Sort by precedence (highest first), then by version (newest first)
  candidates.sort((a, b) => {
    if (b.precedence !== a.precedence) return b.precedence - a.precedence;
    return b.policy.version - a.policy.version;
  });

  return candidates[0].policy;
}

/**
 * Resolve all effective policies for a user across all domains.
 */
export function resolveAllEffectivePolicies(
  policies: PolicyRule[],
  context: PolicyResolutionContext
): Map<PolicyDomain, PolicyRule> {
  const result = new Map<PolicyDomain, PolicyRule>();
  const domains: PolicyDomain[] = [
    'allowed_connectors', 'approved_destinations', 'encryption_requirements',
    'max_transfer_size', 'file_type_restrictions', 'checksum_requirements',
    'naming_strictness', 'sync_conflict_policies', 'deletion_restrictions',
    'ai_usage', 'retention',
  ];

  for (const domain of domains) {
    const effective = resolveEffectivePolicy(domain, policies, context);
    if (effective) {
      result.set(domain, effective);
    }
  }

  return result;
}

/**
 * Check if a policy change would create conflicts.
 */
export function detectPolicyConflicts(
  newPolicy: PolicyRule,
  existingPolicies: PolicyRule[]
): PolicyRule[] {
  return existingPolicies.filter(existing =>
    existing.domain === newPolicy.domain &&
    existing.assignmentTarget === newPolicy.assignmentTarget &&
    existing.assignmentValue === newPolicy.assignmentValue &&
    existing.id !== newPolicy.id &&
    existing.isActive
  );
}

/**
 * Validate policy configuration based on domain.
 */
export function validatePolicyConfig(
  domain: PolicyDomain,
  config: Record<string, unknown>
): { valid: boolean; errors: string[] } {
  const errors: string[] = [];

  switch (domain) {
    case 'max_transfer_size':
      if (typeof config.maxFileSizeMB !== 'number' || config.maxFileSizeMB <= 0) {
        errors.push('Max file size must be a positive number');
      }
      if (typeof config.maxBatchSizeMB !== 'number' || config.maxBatchSizeMB <= 0) {
        errors.push('Max batch size must be a positive number');
      }
      break;

    case 'encryption_requirements':
      if (typeof config.atRest !== 'boolean') {
        errors.push('At-rest encryption must be specified');
      }
      if (typeof config.inTransit !== 'boolean') {
        errors.push('In-transit encryption must be specified');
      }
      break;

    case 'retention':
      if (typeof config.defaultRetentionDays !== 'number' || config.defaultRetentionDays < 1) {
        errors.push('Retention days must be at least 1');
      }
      break;

    case 'file_type_restrictions':
      if (!config.mode || !['allowlist', 'blocklist'].includes(config.mode as string)) {
        errors.push('Mode must be allowlist or blocklist');
      }
      if (!Array.isArray(config.extensions) || config.extensions.length === 0) {
        errors.push('At least one file extension must be specified');
      }
      break;

    case 'naming_strictness':
      if (typeof config.tier !== 'number' || ![1, 2, 3].includes(config.tier as number)) {
        errors.push('Naming tier must be 1, 2, or 3');
      }
      break;
  }

  return { valid: errors.length === 0, errors };
}
