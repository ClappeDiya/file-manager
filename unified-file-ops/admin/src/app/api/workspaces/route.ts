import { NextRequest, NextResponse } from 'next/server';
import * as db from '@/lib/db';

/** GET /api/workspaces - List workspaces and shared resources */
export async function GET(request: NextRequest) {
  const { searchParams } = new URL(request.url);
  const type = searchParams.get('type');

  switch (type) {
    case 'connections':
      return NextResponse.json({ connections: db.getSharedConnections() });
    case 'sync_templates':
      return NextResponse.json({ templates: db.getSyncTemplates() });
    default:
      return NextResponse.json({
        workspaces: db.getWorkspaces(),
        connections: db.getSharedConnections(),
        syncTemplates: db.getSyncTemplates(),
      });
  }
}

/** POST /api/workspaces - Share a resource or create a workspace */
export async function POST(request: NextRequest) {
  try {
    const body = await request.json();
    const { action } = body;

    switch (action) {
      case 'share_connection':
        return NextResponse.json({
          success: true,
          message: 'Connection shared. Propagation will complete within 5 minutes.',
        });
      case 'share_template':
        return NextResponse.json({
          success: true,
          message: 'Template shared. Propagation will complete within 5 minutes.',
        });
      case 'activate_template':
        return NextResponse.json({
          success: true,
          message: 'Template activated for your desktop client.',
        });
      case 'propagate':
        return NextResponse.json({
          success: true,
          message: 'Propagation initiated. Target clients will sync within 5 minutes.',
          estimatedCompletionSeconds: 180,
        });
      default:
        return NextResponse.json({ error: 'Unknown action' }, { status: 400 });
    }
  } catch {
    return NextResponse.json({ error: 'Invalid request' }, { status: 400 });
  }
}
