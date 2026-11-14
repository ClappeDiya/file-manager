/** Governance policies and approval requests, scoped to an organization. */
import { db, newId, now, parseJson, type Row } from '../index';
import type { PolicyRule } from '../../types/policies';
import type { ApprovalRequest, ApprovalState } from '../../types/approvals';

function policyFromRow(r: Row): PolicyRule {
  return {
    id: r.id as string,
    domain: r.domain as PolicyRule['domain'],
    name: r.name as string,
    description: (r.description as string) || '',
    assignmentTarget: r.assignment_target as PolicyRule['assignmentTarget'],
    assignmentValue: (r.assignment_value as string) || '',
    enforcementMode: r.enforcement_mode as PolicyRule['enforcementMode'],
    configuration: parseJson(r.configuration, {}),
    createdBy: (r.created_by as string) || '',
    createdAt: r.created_at as string,
    updatedAt: (r.updated_at as string) || (r.created_at as string),
    version: Number(r.version),
    isActive: Boolean(r.is_active),
  };
}

export function listPolicies(orgId: string): PolicyRule[] {
  return (db().prepare('SELECT * FROM policies WHERE org_id = ? ORDER BY created_at DESC').all(orgId) as Row[]).map(policyFromRow);
}

export function getPolicy(orgId: string, id: string): PolicyRule | null {
  const r = db().prepare('SELECT * FROM policies WHERE org_id = ? AND id = ?').get(orgId, id) as Row | undefined;
  return r ? policyFromRow(r) : null;
}

export function createPolicy(orgId: string, p: Omit<PolicyRule, 'id' | 'createdAt' | 'updatedAt' | 'version'>): PolicyRule {
  const id = newId('pol');
  const ts = now();
  db()
    .prepare(`INSERT INTO policies (id, org_id, domain, name, description, assignment_target, assignment_value, enforcement_mode, configuration, created_by, version, is_active, created_at, updated_at)
              VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?, ?)`)
    .run(id, orgId, p.domain, p.name, p.description ?? '', p.assignmentTarget, p.assignmentValue, p.enforcementMode, JSON.stringify(p.configuration ?? {}), p.createdBy, p.isActive ? 1 : 0, ts, ts);
  return getPolicy(orgId, id)!;
}

export type PolicyUpdate = Partial<Pick<PolicyRule, 'name' | 'description' | 'configuration' | 'enforcementMode' | 'assignmentTarget' | 'assignmentValue' | 'isActive'>>;

export function updatePolicy(orgId: string, id: string, u: PolicyUpdate): PolicyRule | null {
  const cur = getPolicy(orgId, id);
  if (!cur) return null;
  const next = { ...cur, ...u };
  db()
    .prepare(`UPDATE policies SET name = ?, description = ?, configuration = ?, enforcement_mode = ?, assignment_target = ?, assignment_value = ?, is_active = ?, version = version + 1, updated_at = ? WHERE org_id = ? AND id = ?`)
    .run(next.name, next.description, JSON.stringify(next.configuration ?? {}), next.enforcementMode, next.assignmentTarget, next.assignmentValue, next.isActive ? 1 : 0, now(), orgId, id);
  return getPolicy(orgId, id);
}

export function deletePolicy(orgId: string, id: string): boolean {
  return db().prepare('DELETE FROM policies WHERE org_id = ? AND id = ?').run(orgId, id).changes > 0;
}

function approvalFromRow(r: Row): ApprovalRequest {
  const expired = r.state === 'pending' && r.expires_at && new Date(r.expires_at as string) <= new Date();
  return {
    id: r.id as string,
    operation: r.operation as string,
    trigger: r.trigger_type as ApprovalRequest['trigger'],
    source: r.source as string,
    destination: r.destination as string,
    fileCount: Number(r.file_count),
    totalSize: Number(r.total_size),
    reason: (r.reason as string) || '',
    triggeringPolicy: (r.triggering_policy as string) || '',
    state: (expired ? 'expired' : r.state) as ApprovalState,
    requestedBy: (r.requested_by as string) || '',
    requestedByName: (r.requested_by_name as string) || '',
    requestedAt: r.requested_at as string,
    reviewedBy: (r.reviewed_by as string) ?? undefined,
    reviewedByName: (r.reviewed_by_name as string) || undefined,
    reviewedAt: (r.reviewed_at as string) ?? undefined,
    reviewComment: (r.review_comment as string) ?? undefined,
    expiresAt: r.expires_at as string,
    organizationId: (r.organization_id as string) || '',
  } as ApprovalRequest;
}

export function listApprovals(orgId: string): ApprovalRequest[] {
  return (db().prepare('SELECT * FROM approvals WHERE organization_id = ? ORDER BY requested_at DESC').all(orgId) as Row[]).map(approvalFromRow);
}

export function getApproval(orgId: string, id: string): ApprovalRequest | null {
  const r = db().prepare('SELECT * FROM approvals WHERE organization_id = ? AND id = ?').get(orgId, id) as Row | undefined;
  return r ? approvalFromRow(r) : null;
}

export function createApproval(orgId: string, a: Omit<ApprovalRequest, 'id' | 'state' | 'requestedAt' | 'expiresAt' | 'organizationId'>, ttlHours: number): ApprovalRequest {
  const id = newId('apr');
  const at = new Date();
  db()
    .prepare(`INSERT INTO approvals (id, organization_id, operation, trigger_type, source, destination, file_count, total_size, reason, triggering_policy, state, requested_by, requested_by_name, requested_at, expires_at)
              VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', ?, ?, ?, ?)`)
    .run(id, orgId, a.operation, a.trigger, a.source, a.destination, a.fileCount ?? 0, a.totalSize ?? 0, a.reason ?? '', a.triggeringPolicy ?? '', a.requestedBy, a.requestedByName ?? '', at.toISOString(), new Date(at.getTime() + ttlHours * 3_600_000).toISOString());
  return getApproval(orgId, id)!;
}

export function reviewApproval(orgId: string, id: string, approve: boolean, reviewer: { email: string; name: string }, comment: string): ApprovalRequest | null {
  const changed = db()
    .prepare(`UPDATE approvals SET state = ?, reviewed_by = ?, reviewed_by_name = ?, reviewed_at = ?, review_comment = ? WHERE organization_id = ? AND id = ? AND state = 'pending' AND expires_at > ?`)
    .run(approve ? 'approved' : 'denied', reviewer.email, reviewer.name, now(), comment, orgId, id, now()).changes;
  return changed ? getApproval(orgId, id) : null;
}
