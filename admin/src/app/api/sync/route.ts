import { NextRequest, NextResponse } from 'next/server';
import { v4 as uuidv4 } from 'uuid';
import { requireRole, isAuthorized } from '@/lib/utils/require-role';

/**
 * Sync Management API (T-059)
 *
 * GET /api/sync - List sync pairs
 * POST /api/sync - Create a sync pair
 */

interface SyncPair {
  id: string;
  name: string;
  source: string;
  destination: string;
  direction: 'bidirectional' | 'source-to-dest' | 'dest-to-source';
  schedule: string | null;
  status: 'idle' | 'syncing' | 'paused' | 'error';
  last_run: string | null;
  next_run: string | null;
  files_synced: number;
  bytes_synced: number;
  created_at: string;
  updated_at: string;
  created_by: string;
}

const syncPairs: SyncPair[] = [];

function getApiKey(request: NextRequest): string | null {
  const auth = request.headers.get('authorization');
  if (auth?.startsWith('Bearer ')) return auth.slice(7);
  return request.headers.get('x-api-key');
}

/** GET /api/sync - List sync pairs */
export async function GET(request: NextRequest) {
  const apiKey = getApiKey(request);
  if (!apiKey) {
    return NextResponse.json({ error: 'Authentication required' }, { status: 401 });
  }

  const { searchParams } = new URL(request.url);
  const status = searchParams.get('status');

  let result = [...syncPairs];
  if (status) result = result.filter(s => s.status === status);

  return NextResponse.json({ sync_pairs: result, total: result.length });
}

/** POST /api/sync - Create a sync pair */
export async function POST(request: NextRequest) {
  const authResult = await requireRole(request, 'manager');
  if (!isAuthorized(authResult)) return authResult;

  try {
    const body = await request.json();
    const { name, source, destination, direction, schedule } = body;

    if (!name || !source || !destination) {
      return NextResponse.json(
        { error: 'name, source, and destination are required' },
        { status: 400 }
      );
    }

    const validDirections = ['bidirectional', 'source-to-dest', 'dest-to-source'];
    if (direction && !validDirections.includes(direction)) {
      return NextResponse.json(
        { error: `Invalid direction. Must be: ${validDirections.join(', ')}` },
        { status: 400 }
      );
    }

    const pair: SyncPair = {
      id: `sync_${uuidv4().slice(0, 12)}`,
      name,
      source,
      destination,
      direction: direction || 'source-to-dest',
      schedule: schedule || null,
      status: 'idle',
      last_run: null,
      next_run: schedule ? new Date(Date.now() + 3600000).toISOString() : null,
      files_synced: 0,
      bytes_synced: 0,
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
      created_by: authResult.user.email,
    };

    syncPairs.push(pair);
    return NextResponse.json({ sync_pair: pair }, { status: 201 });
  } catch {
    return NextResponse.json({ error: 'Invalid request body' }, { status: 400 });
  }
}
