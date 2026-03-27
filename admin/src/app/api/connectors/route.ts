import { NextRequest, NextResponse } from 'next/server';
import { requireRole, isAuthorized } from '@/lib/utils/require-role';

interface Connector {
  id: string;
  name: string;
  type: string;
  status: 'connected' | 'disconnected' | 'error';
  config: Record<string, string>;
  lastTestedAt?: string;
}

let connectors: Connector[] = [
  { id: 'conn_001', name: 'Production S3', type: 's3', status: 'connected', lastTestedAt: '2026-03-15T08:00:00Z', config: { bucket: 'acme-prod', region: 'us-east-1' } },
  { id: 'conn_002', name: 'Archive Azure', type: 'azure_blob', status: 'connected', lastTestedAt: '2026-03-15T08:00:00Z', config: { container: 'archive', account: 'acmestorage' } },
  { id: 'conn_003', name: 'Partner SFTP', type: 'sftp', status: 'disconnected', config: { host: 'sftp.partner.com', port: '22' } },
];

/** GET /api/connectors - List all connectors */
export async function GET(request: NextRequest) {
  const authResult = await requireRole(request, 'viewer');
  if (!isAuthorized(authResult)) return authResult;

  return NextResponse.json({ connectors });
}

/** POST /api/connectors - Add a new connector */
export async function POST(request: NextRequest) {
  const authResult = await requireRole(request, 'manager');
  if (!isAuthorized(authResult)) return authResult;

  try {
    const body = await request.json();
    const { name, type, config } = body;

    if (!name || !type) {
      return NextResponse.json({ error: 'Name and type required' }, { status: 400 });
    }

    const { v4: uuidv4 } = await import('uuid');
    const newConnector: Connector = {
      id: `conn_${uuidv4().slice(0, 12)}`,
      name,
      type,
      status: 'disconnected',
      config: config || {},
    };

    connectors.push(newConnector);
    return NextResponse.json({ connector: newConnector }, { status: 201 });
  } catch {
    return NextResponse.json({ error: 'Invalid request' }, { status: 400 });
  }
}

/** DELETE /api/connectors - Remove a connector */
export async function DELETE(request: NextRequest) {
  const authResult = await requireRole(request, 'manager');
  if (!isAuthorized(authResult)) return authResult;

  const { searchParams } = new URL(request.url);
  const id = searchParams.get('id');

  if (!id) {
    return NextResponse.json({ error: 'Connector ID required' }, { status: 400 });
  }

  connectors = connectors.filter(c => c.id !== id);
  return NextResponse.json({ success: true });
}
