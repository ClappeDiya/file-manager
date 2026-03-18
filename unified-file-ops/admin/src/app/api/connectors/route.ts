import { NextRequest, NextResponse } from 'next/server';
import * as db from '@/lib/db';

/** GET /api/connectors - List all connectors */
export async function GET() {
  const connectors = db.getConnectors();
  return NextResponse.json({ connectors });
}

/** POST /api/connectors - Add a new connector */
export async function POST(request: NextRequest) {
  try {
    const body = await request.json();
    const { name, type, endpoint, region } = body;

    if (!name || !type) {
      return NextResponse.json({ error: 'Name and type required' }, { status: 400 });
    }

    const newConnector = {
      id: `conn_${Date.now()}`,
      name,
      type,
      status: 'active',
      endpoint: endpoint || undefined,
      region: region || undefined,
      createdAt: new Date().toISOString(),
    };

    db.createConnector(newConnector);
    return NextResponse.json({ connector: newConnector }, { status: 201 });
  } catch {
    return NextResponse.json({ error: 'Invalid request' }, { status: 400 });
  }
}

/** DELETE /api/connectors - Remove a connector */
export async function DELETE(request: NextRequest) {
  const { searchParams } = new URL(request.url);
  const id = searchParams.get('id');

  if (!id) {
    return NextResponse.json({ error: 'Connector ID required' }, { status: 400 });
  }

  db.deleteConnector(id);
  return NextResponse.json({ success: true });
}
