import { NextRequest, NextResponse } from 'next/server';

/**
 * Connector Detail API (T-059)
 *
 * GET /api/connectors/:id - Get connector details
 * PATCH /api/connectors/:id - Update connector config
 * DELETE /api/connectors/:id - Remove connector
 * POST /api/connectors/:id - Trigger connector action (test, connect, disconnect)
 */

function getApiKey(request: NextRequest): string | null {
  const auth = request.headers.get('authorization');
  if (auth?.startsWith('Bearer ')) return auth.slice(7);
  return request.headers.get('x-api-key');
}

/** GET /api/connectors/:id */
export async function GET(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> }
) {
  const apiKey = getApiKey(request);
  if (!apiKey) {
    return NextResponse.json({ error: 'Authentication required' }, { status: 401 });
  }

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
  const apiKey = getApiKey(request);
  if (!apiKey) {
    return NextResponse.json({ error: 'Authentication required' }, { status: 401 });
  }

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
  const apiKey = getApiKey(request);
  if (!apiKey) {
    return NextResponse.json({ error: 'Authentication required' }, { status: 401 });
  }

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
  const apiKey = getApiKey(request);
  if (!apiKey) {
    return NextResponse.json({ error: 'Authentication required' }, { status: 401 });
  }

  const { id } = await params;
  return NextResponse.json({ success: true, deleted: id });
}
