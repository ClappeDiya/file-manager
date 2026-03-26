import { NextRequest, NextResponse } from 'next/server';
import { dataStore } from '@/lib/data-store';
import { requireRole, isAuthorized } from '@/lib/utils/require-role';

/** GET /api/workspaces - List workspaces and shared resources */
export async function GET(request: NextRequest) {
  const { searchParams } = new URL(request.url);
  const type = searchParams.get('type'); // connections, templates, workspaces

  switch (type) {
    case 'connections':
      return NextResponse.json({ connections: dataStore.sharedConnections });
    case 'sync_templates':
      return NextResponse.json({ templates: dataStore.syncTemplates });
    default:
      return NextResponse.json({
        workspaces: dataStore.workspaces,
        connections: dataStore.sharedConnections,
        syncTemplates: dataStore.syncTemplates,
      });
  }
}

/** POST /api/workspaces - Share a resource or create a workspace */
export async function POST(request: NextRequest) {
  const authResult = await requireRole(request, 'manager');
  if (!isAuthorized(authResult)) return authResult;

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
