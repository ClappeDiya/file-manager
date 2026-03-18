import { NextRequest, NextResponse } from 'next/server';
import * as db from '@/lib/db';
import type { PolicyRule, PolicyDomain, PolicyAssignmentTarget } from '@/lib/types/policies';
import { validatePolicyConfig, detectPolicyConflicts } from '@/lib/utils/policy-engine';

/** GET /api/policies - List all policies */
export async function GET(request: NextRequest) {
  const { searchParams } = new URL(request.url);
  const domain = searchParams.get('domain') as PolicyDomain | null;
  const target = searchParams.get('target') as PolicyAssignmentTarget | null;
  const active = searchParams.get('active');

  let result = db.getPolicies();

  if (domain) result = result.filter(p => p.domain === domain);
  if (target) result = result.filter(p => p.assignmentTarget === target);
  if (active !== null) result = result.filter(p => p.isActive === (active === 'true'));

  return NextResponse.json({ policies: result, total: result.length });
}

/** POST /api/policies - Create a new policy */
export async function POST(request: NextRequest) {
  try {
    const body = await request.json();
    const { domain, name, description, assignmentTarget, assignmentValue, enforcementMode, configuration } = body;

    if (!domain || !name || !assignmentTarget || !assignmentValue) {
      return NextResponse.json({ error: 'Missing required fields' }, { status: 400 });
    }

    if (configuration) {
      const validation = validatePolicyConfig(domain, configuration);
      if (!validation.valid) {
        return NextResponse.json({ error: 'Invalid configuration', details: validation.errors }, { status: 400 });
      }
    }

    const newPolicy: PolicyRule = {
      id: `pol_${Date.now()}`,
      domain,
      name,
      description: description || '',
      assignmentTarget,
      assignmentValue,
      enforcementMode: enforcementMode || 'enforce',
      configuration: configuration || {},
      createdBy: 'usr_001',
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
      version: 1,
      isActive: true,
    };

    const conflicts = detectPolicyConflicts(newPolicy, db.getPolicies());
    db.createPolicy(newPolicy);

    return NextResponse.json({
      policy: newPolicy,
      conflicts: conflicts.map(c => ({ id: c.id, name: c.name })),
    }, { status: 201 });
  } catch {
    return NextResponse.json({ error: 'Invalid request' }, { status: 400 });
  }
}

/** PUT /api/policies - Update a policy */
export async function PUT(request: NextRequest) {
  try {
    const body = await request.json();
    const { id, ...updates } = body;

    if (!id) {
      return NextResponse.json({ error: 'Policy ID required' }, { status: 400 });
    }

    const updated = db.updatePolicy(id, updates);
    if (!updated) {
      return NextResponse.json({ error: 'Policy not found' }, { status: 404 });
    }

    return NextResponse.json({ policy: updated });
  } catch {
    return NextResponse.json({ error: 'Invalid request' }, { status: 400 });
  }
}

/** DELETE /api/policies - Delete a policy */
export async function DELETE(request: NextRequest) {
  const { searchParams } = new URL(request.url);
  const id = searchParams.get('id');

  if (!id) {
    return NextResponse.json({ error: 'Policy ID required' }, { status: 400 });
  }

  db.deletePolicy(id);
  return NextResponse.json({ success: true });
}
