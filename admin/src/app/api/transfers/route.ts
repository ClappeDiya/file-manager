import { NextRequest, NextResponse } from 'next/server';
import { v4 as uuidv4 } from 'uuid';
import { requireRole, isAuthorized } from '@/lib/utils/require-role';

/**
 * Transfer Management API (T-059)
 *
 * GET /api/transfers - List transfers (filterable by status)
 * POST /api/transfers - Trigger a new transfer
 */

interface Transfer {
  id: string;
  source: string;
  destination: string;
  status: 'queued' | 'active' | 'paused' | 'completed' | 'failed' | 'cancelled';
  progress: number;
  total_bytes: number;
  transferred_bytes: number;
  speed_bps: number;
  priority: 'high' | 'normal' | 'low';
  created_at: string;
  updated_at: string;
  error_message: string | null;
  verify_checksum: boolean;
  created_by: string;
}

const transfers: Transfer[] = [];

/** GET /api/transfers - List transfers */
export async function GET(request: NextRequest) {
  const authResult = await requireRole(request, 'viewer');
  if (!isAuthorized(authResult)) return authResult;

  const { searchParams } = new URL(request.url);
  const status = searchParams.get('status');
  const priority = searchParams.get('priority');
  const limit = parseInt(searchParams.get('limit') || '50');
  const offset = parseInt(searchParams.get('offset') || '0');

  let result = [...transfers];

  if (status) result = result.filter(t => t.status === status);
  if (priority) result = result.filter(t => t.priority === priority);

  const total = result.length;
  result = result.slice(offset, offset + limit);

  return NextResponse.json({
    transfers: result,
    total,
    limit,
    offset,
    has_more: offset + limit < total,
  });
}

/** POST /api/transfers - Trigger a new transfer */
export async function POST(request: NextRequest) {
  const authResult = await requireRole(request, 'manager');
  if (!isAuthorized(authResult)) return authResult;

  try {
    const body = await request.json();
    const { source, destination, priority, verify_checksum } = body;

    if (!source || !destination) {
      return NextResponse.json(
        { error: 'source and destination are required' },
        { status: 400 }
      );
    }

    const transfer: Transfer = {
      id: `xfr_${uuidv4().slice(0, 12)}`,
      source,
      destination,
      status: 'queued',
      progress: 0,
      total_bytes: 0,
      transferred_bytes: 0,
      speed_bps: 0,
      priority: priority || 'normal',
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
      error_message: null,
      verify_checksum: verify_checksum ?? true,
      created_by: authResult.user.email,
    };

    transfers.push(transfer);

    return NextResponse.json({ transfer }, { status: 201 });
  } catch {
    return NextResponse.json({ error: 'Invalid request body' }, { status: 400 });
  }
}
