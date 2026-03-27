import { NextRequest, NextResponse } from 'next/server';
import { dataStore } from '@/lib/data-store';
import type { PolicyRule, PolicyDomain, PolicyAssignmentTarget } from '@/lib/types/policies';
import { validatePolicyConfig, detectPolicyConflicts } from '@/lib/utils/policy-engine';
import { requireRole, isAuthorized } from '@/lib/utils/require-role';

/** GET /api/policies - List all policies */
export async function GET(request: NextRequest) {
  const authResult = await requireRole(request, 'viewer');
  if (!isAuthorized(authResult)) return authResult;

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
  const authResult = await requireRole(request, 'manager');
  if (!isAuthorized(authResult)) return authResult;

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

    const { v4: uuidv4 } = await import('uuid');
    const newPolicy: PolicyRule = {
      id: `pol_${uuidv4().slice(0, 12)}`,
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
  const authResult = await requireRole(request, 'manager');
  if (!isAuthorized(authResult)) return authResult;

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

    // Whitelist allowed update fields to prevent mass assignment
    const ALLOWED_UPDATE_FIELDS = ['name', 'description', 'configuration', 'enforcementMode', 'assignmentTarget', 'assignmentValue', 'isActive'] as const;
    const safeUpdates: Record<string, unknown> = {};
    for (const field of ALLOWED_UPDATE_FIELDS) {
      if (field in updates) {
        safeUpdates[field] = updates[field];
      }
    }

    dataStore.policies[index] = {
      ...dataStore.policies[index],
      ...safeUpdates,
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
  const authResult = await requireRole(request, 'manager');
  if (!isAuthorized(authResult)) return authResult;

  const { searchParams } = new URL(request.url);
  const id = searchParams.get('id');

  if (!id) {
    return NextResponse.json({ error: 'Policy ID required' }, { status: 400 });
  }

  dataStore.policies = dataStore.policies.filter(p => p.id !== id);
  return NextResponse.json({ success: true });
}
