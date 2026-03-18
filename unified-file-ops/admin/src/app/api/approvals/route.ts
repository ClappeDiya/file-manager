import { NextRequest, NextResponse } from 'next/server';
import * as db from '@/lib/db';
import type { ApprovalRequest, ApprovalState, ApprovalTrigger } from '@/lib/types/approvals';

/** GET /api/approvals - List approval requests */
export async function GET(request: NextRequest) {
  const { searchParams } = new URL(request.url);
  const state = searchParams.get('state') as ApprovalState | null;
  const trigger = searchParams.get('trigger') as ApprovalTrigger | null;

  // Auto-expire stale approvals in DB
  db.expireApprovals();

  let result = db.getApprovals();

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
    const expiresAt = new Date(now.getTime() + 48 * 60 * 60 * 1000);

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

    db.createApproval(newApproval);
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

    const existing = db.getApprovals().find(a => a.id === id);
    if (!existing) {
      return NextResponse.json({ error: 'Approval not found' }, { status: 404 });
    }

    if (existing.state !== 'pending') {
      return NextResponse.json({ error: 'Can only review pending approvals' }, { status: 400 });
    }

    if (new Date(existing.expiresAt) <= new Date()) {
      db.updateApproval(id, { state: 'expired' });
      return NextResponse.json({ error: 'Approval has expired' }, { status: 400 });
    }

    const updates: Partial<ApprovalRequest> = {
      state: action === 'approve' ? 'approved' : 'denied',
      reviewedBy: reviewedBy || 'unknown',
      reviewedByName: reviewedByName || 'Unknown',
      reviewedAt: new Date().toISOString(),
      reviewComment: reviewComment || '',
    };

    db.updateApproval(id, updates);

    return NextResponse.json({ approval: { ...existing, ...updates } });
  } catch {
    return NextResponse.json({ error: 'Invalid request' }, { status: 400 });
  }
}
