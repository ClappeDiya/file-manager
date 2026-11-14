import { NextRequest, NextResponse } from 'next/server';
import type { PolicyDomain, PolicyAssignmentTarget, PolicyRule } from '@/lib/types/policies';
import { validatePolicyConfig, detectPolicyConflicts } from '@/lib/utils/policy-engine';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { createPolicy, deletePolicy, listPolicies, updatePolicy, type PolicyUpdate } from '@/lib/db/repos/governance';

/** GET /api/policies - List the current organization's policies */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'policies.read');
  if (!isContext(ctx)) return ctx;
  const { searchParams } = new URL(request.url);
  const domain = searchParams.get('domain') as PolicyDomain | null;
  const target = searchParams.get('target') as PolicyAssignmentTarget | null;
  const active = searchParams.get('active');
  let result = listPolicies(ctx.orgId);
  if (domain) result = result.filter((p) => p.domain === domain);
  if (target) result = result.filter((p) => p.assignmentTarget === target);
  if (active !== null) result = result.filter((p) => p.isActive === (active === 'true'));
  return NextResponse.json({ policies: result, total: result.length });
}

/** POST /api/policies - Create a policy */
export async function POST(request: NextRequest) {
  const ctx = await requirePermission(request, 'policies.write');
  if (!isContext(ctx)) return ctx;
  const body = await readJson<Partial<PolicyRule>>(request);
  if (!body) return badRequest('Invalid request');
  const { domain, name, description, assignmentTarget, assignmentValue, enforcementMode, configuration } = body;
  if (!domain || !name || !assignmentTarget || !assignmentValue) return badRequest('Missing required fields');
  if (configuration) {
    const v = validatePolicyConfig(domain, configuration);
    if (!v.valid) return NextResponse.json({ error: 'Invalid configuration', details: v.errors }, { status: 400 });
  }
  const draft = { domain, name, description: description || '', assignmentTarget, assignmentValue, enforcementMode: enforcementMode || 'enforce', configuration: configuration || {}, createdBy: ctx.user.id, isActive: true } as const;
  const conflicts = detectPolicyConflicts({ ...draft, id: '', createdAt: '', updatedAt: '', version: 1 } as PolicyRule, listPolicies(ctx.orgId));
  const policy = createPolicy(ctx.orgId, draft);
  audit(ctx, { eventType: 'policy.create', description: `Created policy "${policy.name}"`, resourceType: 'policy', resourceId: policy.id });
  return NextResponse.json({ policy, conflicts: conflicts.map((c) => ({ id: c.id, name: c.name })) }, { status: 201 });
}

/** PUT /api/policies - Update a policy (whitelisted fields) */
export async function PUT(request: NextRequest) {
  const ctx = await requirePermission(request, 'policies.write');
  if (!isContext(ctx)) return ctx;
  const body = await readJson<Record<string, unknown>>(request);
  if (!body || typeof body.id !== 'string') return badRequest('Policy ID required');
  const allowed = ['name', 'description', 'configuration', 'enforcementMode', 'assignmentTarget', 'assignmentValue', 'isActive'] as const;
  const updates: PolicyUpdate = {};
  for (const f of allowed) if (f in body) (updates as Record<string, unknown>)[f] = body[f];
  const policy = updatePolicy(ctx.orgId, body.id, updates);
  if (!policy) return badRequest('Policy not found', 404);
  audit(ctx, { eventType: 'policy.update', description: `Updated policy "${policy.name}"`, resourceType: 'policy', resourceId: policy.id, metadata: { fields: Object.keys(updates) } });
  return NextResponse.json({ policy });
}

/** DELETE /api/policies?id= - Delete a policy */
export async function DELETE(request: NextRequest) {
  const ctx = await requirePermission(request, 'policies.write');
  if (!isContext(ctx)) return ctx;
  const id = new URL(request.url).searchParams.get('id');
  if (!id) return badRequest('Policy ID required');
  if (!deletePolicy(ctx.orgId, id)) return badRequest('Policy not found', 404);
  audit(ctx, { eventType: 'policy.delete', description: `Deleted policy ${id}`, resourceType: 'policy', resourceId: id });
  return NextResponse.json({ success: true });
}
