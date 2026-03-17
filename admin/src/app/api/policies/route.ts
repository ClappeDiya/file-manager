import { NextRequest, NextResponse } from 'next/server';
import { dataStore } from '@/lib/data-store';
import type { PolicyRule, PolicyDomain, PolicyAssignmentTarget } from '@/lib/types/policies';
import { validatePolicyConfig, detectPolicyConflicts } from '@/lib/utils/policy-engine';

/** GET /api/policies - List all policies */
export async function GET(request: NextRequest) {
  const { searchParams } = new URL(request.url);
  const domain = searchParams.get('domain') as PolicyDomain | null;
  const target = searchParams.get('target') as PolicyAssignmentTarget | null;
  const active = searchParams.get('active');

  let result = [...dataStore.policies];

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

    // Validate config
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
      createdBy: 'usr_001', // from auth context in production
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
      version: 1,
      isActive: true,
    };

    // Check for conflicts
    const conflicts = detectPolicyConflicts(newPolicy, dataStore.policies);

    dataStore.policies.push(newPolicy);

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

    const index = dataStore.policies.findIndex(p => p.id === id);
    if (index === -1) {
      return NextResponse.json({ error: 'Policy not found' }, { status: 404 });
    }

    dataStore.policies[index] = {
      ...dataStore.policies[index],
      ...updates,
      updatedAt: new Date().toISOString(),
      version: dataStore.policies[index].version + 1,
    };

    return NextResponse.json({ policy: dataStore.policies[index] });
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

  dataStore.policies = dataStore.policies.filter(p => p.id !== id);
  return NextResponse.json({ success: true });
}
