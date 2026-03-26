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

// Rate limiting state
const rateLimits: Map<string, { count: number; resetAt: number }> = new Map();
const RATE_LIMIT = 1000; // requests per minute
const RATE_WINDOW = 60000; // 1 minute in ms

function checkRateLimit(apiKey: string): boolean {
  const now = Date.now();
  const entry = rateLimits.get(apiKey);
  if (!entry || now > entry.resetAt) {
    rateLimits.set(apiKey, { count: 1, resetAt: now + RATE_WINDOW });
    return true;
  }
  if (entry.count >= RATE_LIMIT) return false;
  entry.count++;
  return true;
}

function getApiKey(request: NextRequest): string | null {
  const auth = request.headers.get('authorization');
  if (auth?.startsWith('Bearer ')) return auth.slice(7);
  return request.headers.get('x-api-key');
}

function unauthorized() {
  return NextResponse.json(
    { error: 'Authentication required', code: 'AUTH_REQUIRED' },
    { status: 401 }
  );
}

function rateLimited() {
  return NextResponse.json(
    { error: 'Rate limit exceeded', code: 'RATE_LIMITED', retry_after: 60 },
    { status: 429, headers: { 'Retry-After': '60' } }
  );
}

/** GET /api/transfers - List transfers */
export async function GET(request: NextRequest) {
  const apiKey = getApiKey(request);
  if (!apiKey) return unauthorized();
  if (!checkRateLimit(apiKey)) return rateLimited();

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

  const apiKey = getApiKey(request);
  if (!apiKey) return unauthorized();
  if (!checkRateLimit(apiKey)) return rateLimited();

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
      created_by: apiKey.slice(0, 8),
    };

    transfers.push(transfer);

    return NextResponse.json({ transfer }, { status: 201 });
  } catch {
    return NextResponse.json({ error: 'Invalid request body' }, { status: 400 });
  }
}
