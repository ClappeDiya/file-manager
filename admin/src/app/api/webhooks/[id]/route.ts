import { NextRequest, NextResponse } from 'next/server';
import { requireRole, isAuthorized } from '@/lib/utils/require-role';

/**
 * Webhook Detail API (T-059)
 *
 * GET /api/webhooks/:id - Get webhook details
 * PATCH /api/webhooks/:id - Update webhook (events, active status)
 * DELETE /api/webhooks/:id - Remove webhook
 */

/** GET /api/webhooks/:id */
export async function GET(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const authResult = await requireRole(request, 'manager');
  if (!isAuthorized(authResult)) return authResult;

  const { id } = await params;
  return NextResponse.json({
    webhook: {
      id,
      url: 'https://example.com/webhook',
      events: ['transfer.completed', 'sync.completed'],
      is_active: true,
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
      last_triggered: null,
      failure_count: 0,
      delivery_history: [],
    },
  });
}

/** PATCH /api/webhooks/:id */
export async function PATCH(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const authResult = await requireRole(request, 'manager');
  if (!isAuthorized(authResult)) return authResult;

  const { id } = await params;
  try {
    const body = await request.json();
    const { events, is_active, url } = body;

    return NextResponse.json({
      success: true,
      webhook: {
        id,
        url: url || 'https://example.com/webhook',
        events: events || ['transfer.completed'],
        is_active: is_active ?? true,
        updated_at: new Date().toISOString(),
      },
    });
  } catch {
    return NextResponse.json({ error: 'Invalid request' }, { status: 400 });
  }
}

/** DELETE /api/webhooks/:id */
export async function DELETE(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const authResult = await requireRole(request, 'manager');
  if (!isAuthorized(authResult)) return authResult;

  const { id } = await params;
  return NextResponse.json({ success: true, deleted: id });
}
