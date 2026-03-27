import { NextRequest, NextResponse } from 'next/server';
import { requireRole, isAuthorized } from '@/lib/utils/require-role';

/**
 * Sync Pair Detail API (T-059)
 *
 * GET /api/sync/:id - Get sync pair details
 * PATCH /api/sync/:id - Update sync pair
 * DELETE /api/sync/:id - Delete sync pair
 * POST /api/sync/:id/run - Trigger sync run (handled via action in PATCH)
 */

/** GET /api/sync/:id */
export async function GET(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const authResult = await requireRole(request, 'viewer');
  if (!isAuthorized(authResult)) return authResult;

  const { id } = await params;
  return NextResponse.json({
    sync_pair: {
      id,
      name: 'Demo Sync',
      source: '/data/source',
      destination: '/data/backup',
      direction: 'source-to-dest',
      schedule: '0 */6 * * *',
      status: 'idle',
      last_run: null,
      next_run: new Date(Date.now() + 3600000).toISOString(),
      files_synced: 0,
      bytes_synced: 0,
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    },
  });
}

/** PATCH /api/sync/:id */
export async function PATCH(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const authResult = await requireRole(request, 'manager');
  if (!isAuthorized(authResult)) return authResult;

  const { id } = await params;
  try {
    const body = await request.json();
    const { action, schedule, direction } = body;

    // Handle run action
    if (action === 'run') {
      return NextResponse.json({
        success: true,
        sync_run: {
          id: `run_${Date.now()}`,
          sync_pair_id: id,
          status: 'running',
          started_at: new Date().toISOString(),
          files_checked: 0,
          files_synced: 0,
          bytes_synced: 0,
        },
      });
    }

    if (action === 'pause') {
      return NextResponse.json({
        success: true,
        sync_pair: { id, status: 'paused', updated_at: new Date().toISOString() },
      });
    }

    // Update configuration
    return NextResponse.json({
      success: true,
      sync_pair: {
        id,
        schedule: schedule || null,
        direction: direction || 'source-to-dest',
        updated_at: new Date().toISOString(),
      },
    });
  } catch {
    return NextResponse.json({ error: 'Invalid request' }, { status: 400 });
  }
}

/** DELETE /api/sync/:id */
export async function DELETE(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const authResult = await requireRole(request, 'manager');
  if (!isAuthorized(authResult)) return authResult;

  const { id } = await params;
  return NextResponse.json({ success: true, deleted: id });
}
