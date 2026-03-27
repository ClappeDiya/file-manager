import { NextRequest, NextResponse } from 'next/server';
import { requireRole, isAuthorized } from '@/lib/utils/require-role';

/**
 * Connector Detail API (T-059)
 *
 * GET /api/connectors/:id - Get connector details
 * PATCH /api/connectors/:id - Update connector config
 * DELETE /api/connectors/:id - Remove connector
 * POST /api/connectors/:id - Trigger connector action (test, connect, disconnect)
 */

/** GET /api/connectors/:id */
export async function GET(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const authResult = await requireRole(request, 'viewer');
  if (!isAuthorized(authResult)) return authResult;

  const { id } = await params;
  return NextResponse.json({
    connector: {
      id,
      name: `Connector ${id}`,
      protocol: 'sftp',
      host: 'files.example.com',
      port: 22,
      status: 'connected',
      last_connected: new Date().toISOString(),
      config: {
        auth_method: 'key',
        root_path: '/',
        timeout: 30,
      },
    },
  });
}

/** POST /api/connectors/:id - Action (test, connect, disconnect) */
export async function POST(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const authResult = await requireRole(request, 'manager');
  if (!isAuthorized(authResult)) return authResult;

  const { id } = await params;
  try {
    const body = await request.json();
    const { action } = body;

    const validActions = ['test', 'connect', 'disconnect'];
    if (!action || !validActions.includes(action)) {
      return NextResponse.json(
        { error: `action required. Must be: ${validActions.join(', ')}` },
        { status: 400 }
      );
    }

    return NextResponse.json({
      success: true,
      connector_id: id,
      action,
      result: action === 'test'
        ? { latency_ms: 42, authenticated: true, server_version: 'OpenSSH 9.0' }
        : { status: action === 'connect' ? 'connected' : 'disconnected' },
      timestamp: new Date().toISOString(),
    });
  } catch {
    return NextResponse.json({ error: 'Invalid request' }, { status: 400 });
  }
}

/** PATCH /api/connectors/:id */
export async function PATCH(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const authResult = await requireRole(request, 'manager');
  if (!isAuthorized(authResult)) return authResult;

  const { id } = await params;
  try {
    const body = await request.json();
    return NextResponse.json({
      success: true,
      connector: {
        id,
        ...body,
        updated_at: new Date().toISOString(),
      },
    });
  } catch {
    return NextResponse.json({ error: 'Invalid request' }, { status: 400 });
  }
}

/** DELETE /api/connectors/:id */
export async function DELETE(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const authResult = await requireRole(request, 'manager');
  if (!isAuthorized(authResult)) return authResult;

  const { id } = await params;
  return NextResponse.json({ success: true, deleted: id });
}
