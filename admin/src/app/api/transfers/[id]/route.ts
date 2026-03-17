import { NextRequest, NextResponse } from 'next/server';

/**
 * Transfer Detail API (T-059)
 *
 * GET /api/transfers/:id - Get transfer details
 * PATCH /api/transfers/:id - Update transfer (pause/resume/cancel/priority)
 * DELETE /api/transfers/:id - Cancel and remove transfer
 */

// Simple in-memory store (shared with parent route via imports would need a shared module)
// In production, this would use a database

function getApiKey(request: NextRequest): string | null {
  const auth = request.headers.get('authorization');
  if (auth?.startsWith('Bearer ')) return auth.slice(7);
  return request.headers.get('x-api-key');
}

/** GET /api/transfers/:id */
export async function GET(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const apiKey = getApiKey(request);
  if (!apiKey) {
    return NextResponse.json({ error: 'Authentication required' }, { status: 401 });
  }

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
  const apiKey = getApiKey(request);
  if (!apiKey) {
    return NextResponse.json({ error: 'Authentication required' }, { status: 401 });
  }

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
  const apiKey = getApiKey(request);
  if (!apiKey) {
    return NextResponse.json({ error: 'Authentication required' }, { status: 401 });
  }

  const { id } = await params;
  return NextResponse.json({
    success: true,
    deleted: id,
  });
}
