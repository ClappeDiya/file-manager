import { NextRequest, NextResponse } from 'next/server';
import { requireRole, isAuthorized } from '@/lib/utils/require-role';

/**
 * Transfer Detail API (T-059)
 *
 * GET /api/transfers/:id - Get transfer details
 * PATCH /api/transfers/:id - Update transfer (pause/resume/cancel/priority)
 * DELETE /api/transfers/:id - Cancel and remove transfer
 */

/** GET /api/transfers/:id */
export async function GET(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const authResult = await requireRole(request, 'viewer');
  if (!isAuthorized(authResult)) return authResult;

  const { id } = await params;

  // Mock response for demo
  return NextResponse.json({
    transfer: {
      id,
      source: '/data/source',
      destination: '/data/dest',
      status: 'active',
      progress: 45.5,
      total_bytes: 1048576,
      transferred_bytes: 476839,
      speed_bps: 52428800,
      priority: 'normal',
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
      error_message: null,
      verify_checksum: true,
    },
  });
}

/** PATCH /api/transfers/:id - Update transfer */
export async function PATCH(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const authResult = await requireRole(request, 'manager');
  if (!isAuthorized(authResult)) return authResult;

  const { id } = await params;
  try {
    const body = await request.json();
    const { action, priority } = body;

    const validActions = ['pause', 'resume', 'cancel', 'retry'];
    if (action && !validActions.includes(action)) {
      return NextResponse.json(
        { error: `Invalid action. Must be one of: ${validActions.join(', ')}` },
        { status: 400 }
      );
    }

    return NextResponse.json({
      success: true,
      transfer: {
        id,
        status: action === 'pause' ? 'paused' : action === 'cancel' ? 'cancelled' : 'active',
        priority: priority || 'normal',
        updated_at: new Date().toISOString(),
      },
    });
  } catch {
    return NextResponse.json({ error: 'Invalid request' }, { status: 400 });
  }
}

/** DELETE /api/transfers/:id */
export async function DELETE(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const authResult = await requireRole(request, 'manager');
  if (!isAuthorized(authResult)) return authResult;

  const { id } = await params;
  return NextResponse.json({
    success: true,
    deleted: id,
  });
}
