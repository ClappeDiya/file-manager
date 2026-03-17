import { NextRequest, NextResponse } from 'next/server';
import { dataStore } from '@/lib/data-store';
import type { ApprovalRequest, ApprovalState, ApprovalTrigger } from '@/lib/types/approvals';

/** GET /api/approvals - List approval requests */
export async function GET(request: NextRequest) {
  const { searchParams } = new URL(request.url);
  const state = searchParams.get('state') as ApprovalState | null;
  const trigger = searchParams.get('trigger') as ApprovalTrigger | null;

  let result = [...dataStore.approvals];

  // Auto-expire stale approvals
  const now = new Date();
  result = result.map(a => {
    if (a.state === 'pending' && new Date(a.expiresAt) <= now) {
      return { ...a, state: 'expired' as ApprovalState };
    }
    return a;
  });

  if (state) result = result.filter(a => a.state === state);
  if (trigger) result = result.filter(a => a.trigger === trigger);

  const stats = {
    pending: result.filter(a => a.state === 'pending').length,
    approved: result.filter(a => a.state === 'approved').length,
    denied: result.filter(a => a.state === 'denied').length,
    expired: result.filter(a => a.state === 'expired').length,
  };

  return NextResponse.json({ approvals: result, stats, total: result.length });
}

/** POST /api/approvals - Create a new approval request */
export async function POST(request: NextRequest) {
  try {
    const body = await request.json();
    const {
      operation, trigger, source, destination,
      fileCount, totalSize, reason, triggeringPolicy,
      requestedBy, requestedByName,
    } = body;

    if (!operation || !trigger || !source || !destination) {
      return NextResponse.json({ error: 'Missing required fields' }, { status: 400 });
    }

    const now = new Date();
    const expiresAt = new Date(now.getTime() + 48 * 60 * 60 * 1000); // 48 hours

    const newApproval: ApprovalRequest = {
      id: `apr_${Date.now()}`,
      operation,
      trigger,
      source,
      destination,
      fileCount: fileCount || 0,
      totalSize: totalSize || 0,
      reason: reason || '',
      triggeringPolicy: triggeringPolicy || '',
      state: 'pending',
      requestedBy: requestedBy || 'unknown',
      requestedByName: requestedByName || 'Unknown User',
      requestedAt: now.toISOString(),
      expiresAt: expiresAt.toISOString(),
      organizationId: 'org_001',
    };

    dataStore.approvals.push(newApproval);
    return NextResponse.json({ approval: newApproval }, { status: 201 });
  } catch {
    return NextResponse.json({ error: 'Invalid request' }, { status: 400 });
  }
}

/** PATCH /api/approvals - Review (approve/deny) a request */
export async function PATCH(request: NextRequest) {
  try {
    const body = await request.json();
    const { id, action, reviewedBy, reviewedByName, reviewComment } = body;

    if (!id || !action || !['approve', 'deny'].includes(action)) {
      return NextResponse.json({ error: 'ID and action (approve/deny) required' }, { status: 400 });
    }

    const index = dataStore.approvals.findIndex(a => a.id === id);
    if (index === -1) {
      return NextResponse.json({ error: 'Approval not found' }, { status: 404 });
    }

    if (dataStore.approvals[index].state !== 'pending') {
      return NextResponse.json({ error: 'Can only review pending approvals' }, { status: 400 });
    }

    // Check expiry
    if (new Date(dataStore.approvals[index].expiresAt) <= new Date()) {
      dataStore.approvals[index].state = 'expired';
      return NextResponse.json({ error: 'Approval has expired' }, { status: 400 });
    }

    dataStore.approvals[index] = {
      ...dataStore.approvals[index],
      state: action === 'approve' ? 'approved' : 'denied',
      reviewedBy: reviewedBy || 'unknown',
      reviewedByName: reviewedByName || 'Unknown',
      reviewedAt: new Date().toISOString(),
      reviewComment: reviewComment || '',
    };

    return NextResponse.json({ approval: dataStore.approvals[index] });
  } catch {
    return NextResponse.json({ error: 'Invalid request' }, { status: 400 });
  }
}
