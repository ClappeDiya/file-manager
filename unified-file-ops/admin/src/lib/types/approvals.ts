/**
 * Approval workflow types (T-051).
 * Defines triggers, request structure, states, and audit trail.
 */

/** What triggers an approval request */
export type ApprovalTrigger =
  | 'external_dest_transfer'
  | 'destructive_sync'
  | 'policy_exception'
  | 'tier3_naming'
  | 'sensitive_file';

export const APPROVAL_TRIGGER_LABELS: Record<ApprovalTrigger, string> = {
  external_dest_transfer: 'External Destination Transfer',
  destructive_sync: 'Destructive Sync Operation',
  policy_exception: 'Policy Exception Request',
  tier3_naming: 'Tier 3 Naming Convention',
  sensitive_file: 'Sensitive File Access',
};

/** Approval request states */
export type ApprovalState = 'pending' | 'approved' | 'denied' | 'expired';

export const APPROVAL_STATE_LABELS: Record<ApprovalState, string> = {
  pending: 'Pending Review',
  approved: 'Approved',
  denied: 'Denied',
  expired: 'Expired',
};

/** Expiration time in hours */
export const APPROVAL_EXPIRY_HOURS = 48;

export interface ApprovalRequest {
  id: string;
  /** The operation being requested */
  operation: string;
  /** What triggered this approval */
  trigger: ApprovalTrigger;
  /** Source location/connector */
  source: string;
  /** Destination location/connector */
  destination: string;
  /** Number of files involved */
  fileCount: number;
  /** Total size in bytes */
  totalSize: number;
  /** User-provided reason for the request */
  reason: string;
  /** Policy that triggered the approval requirement */
  triggeringPolicy: string;
  /** Current state */
  state: ApprovalState;
  /** Who requested the approval */
  requestedBy: string;
  requestedByName: string;
  requestedAt: string;
  /** Who acted on the request */
  reviewedBy?: string;
  reviewedByName?: string;
  reviewedAt?: string;
  /** Reviewer's comment */
  reviewComment?: string;
  /** Auto-expires after 48 hours */
  expiresAt: string;
  /** Organization */
  organizationId: string;
}

export interface ApprovalAuditEntry {
  id: string;
  approvalId: string;
  action: 'created' | 'approved' | 'denied' | 'expired' | 'notified' | 'escalated';
  performedBy: string;
  performedByName: string;
  performedAt: string;
  details: string;
  metadata?: Record<string, unknown>;
}

export interface ApprovalNotification {
  id: string;
  approvalId: string;
  recipientId: string;
  channel: 'in_app' | 'email';
  sentAt: string;
  readAt?: string;
}

export interface ApprovalStats {
  pending: number;
  approvedToday: number;
  deniedToday: number;
  expiredToday: number;
  averageResponseTimeMinutes: number;
}
