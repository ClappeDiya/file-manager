/**
 * Policy engine types (T-050).
 * Covers all 11 policy domains with assignment and precedence.
 */

/** All 11 policy domains */
export type PolicyDomain =
  | 'allowed_connectors'
  | 'approved_destinations'
  | 'encryption_requirements'
  | 'max_transfer_size'
  | 'file_type_restrictions'
  | 'checksum_requirements'
  | 'naming_strictness'
  | 'sync_conflict_policies'
  | 'deletion_restrictions'
  | 'ai_usage'
  | 'retention';

export const POLICY_DOMAIN_LABELS: Record<PolicyDomain, string> = {
  allowed_connectors: 'Allowed Connectors',
  approved_destinations: 'Approved Destinations',
  encryption_requirements: 'Encryption Requirements',
  max_transfer_size: 'Max Transfer Size',
  file_type_restrictions: 'File Type Restrictions',
  checksum_requirements: 'Checksum Requirements',
  naming_strictness: 'Naming Strictness',
  sync_conflict_policies: 'Sync Conflict Policies',
  deletion_restrictions: 'Deletion Restrictions',
  ai_usage: 'AI Usage Policy',
  retention: 'Retention Policy',
};

export const POLICY_DOMAIN_DESCRIPTIONS: Record<PolicyDomain, string> = {
  allowed_connectors: 'Control which cloud/storage connectors can be used',
  approved_destinations: 'Restrict transfer destinations to approved endpoints',
  encryption_requirements: 'Enforce encryption for data at rest and in transit',
  max_transfer_size: 'Set maximum file or batch size for transfers',
  file_type_restrictions: 'Block or allow specific file types/extensions',
  checksum_requirements: 'Require checksum verification for transfers',
  naming_strictness: 'Enforce file naming conventions and rules',
  sync_conflict_policies: 'Define how sync conflicts are resolved',
  deletion_restrictions: 'Control who can delete files and under what conditions',
  ai_usage: 'Govern how AI features can process organizational data',
  retention: 'Set data retention periods and auto-cleanup rules',
};

/** Where a policy is assigned */
export type PolicyAssignmentTarget = 'org' | 'role' | 'user' | 'connection_type';

/** Precedence: user > role > org (higher number = higher priority) */
export const POLICY_PRECEDENCE: Record<PolicyAssignmentTarget, number> = {
  user: 30,
  role: 20,
  org: 10,
  connection_type: 15,
};

/** Enforcement mode */
export type PolicyEnforcementMode = 'enforce' | 'warn' | 'audit_only' | 'disabled';

export interface PolicyRule {
  id: string;
  domain: PolicyDomain;
  name: string;
  description: string;
  assignmentTarget: PolicyAssignmentTarget;
  assignmentValue: string; // org ID, role name, user ID, or connection type
  enforcementMode: PolicyEnforcementMode;
  configuration: Record<string, unknown>;
  createdBy: string;
  createdAt: string;
  updatedAt: string;
  version: number;
  isActive: boolean;
}

/** Domain-specific configuration types */
export interface AllowedConnectorsConfig {
  allowed: string[];
  blocked: string[];
}

export interface ApprovedDestinationsConfig {
  allowedDomains: string[];
  allowedPaths: string[];
  blockedDomains: string[];
}

export interface EncryptionConfig {
  atRest: boolean;
  inTransit: boolean;
  algorithm: 'AES-256' | 'AES-128' | 'ChaCha20';
  keyRotationDays: number;
}

export interface MaxTransferSizeConfig {
  maxFileSizeMB: number;
  maxBatchSizeMB: number;
  maxFileCount: number;
}

export interface FileTypeRestrictionsConfig {
  mode: 'allowlist' | 'blocklist';
  extensions: string[];
  mimeTypes: string[];
}

export interface ChecksumConfig {
  required: boolean;
  algorithm: 'SHA-256' | 'SHA-512' | 'MD5' | 'CRC32';
  verifyOnReceive: boolean;
}

export interface NamingStrictnessConfig {
  enforceConventions: boolean;
  allowedPatterns: string[];
  maxLength: number;
  disallowSpecialChars: boolean;
  tier: 1 | 2 | 3;
}

export interface SyncConflictConfig {
  resolution: 'newest_wins' | 'oldest_wins' | 'manual' | 'duplicate';
  notifyOnConflict: boolean;
  keepConflictCopies: boolean;
}

export interface DeletionRestrictionsConfig {
  allowPermanentDelete: boolean;
  requireApproval: boolean;
  softDeleteRetentionDays: number;
  protectedPaths: string[];
}

export interface AIUsageConfig {
  allowClassification: boolean;
  allowContentAnalysis: boolean;
  allowNamingSuggestions: boolean;
  dataResidency: 'local' | 'cloud' | 'hybrid';
  excludedPaths: string[];
}

export interface RetentionConfig {
  defaultRetentionDays: number;
  archiveAfterDays: number;
  autoDeleteAfterDays: number;
  excludedPaths: string[];
}

/** Policy propagation status */
export interface PolicyPropagation {
  policyId: string;
  lastPropagatedAt: string;
  targetDeviceCount: number;
  confirmedDeviceCount: number;
  failedDeviceCount: number;
  status: 'propagating' | 'complete' | 'partial' | 'failed';
}
