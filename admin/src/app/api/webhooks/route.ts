import { NextRequest, NextResponse } from 'next/server';
import { v4 as uuidv4 } from 'uuid';
import { requireRole, isAuthorized } from '@/lib/utils/require-role';

/**
 * Webhook Management API (T-059)
 *
 * GET /api/webhooks - List registered webhooks
 * POST /api/webhooks - Register a new webhook
 *
 * Events: transfer.started, transfer.completed, transfer.failed,
 *         sync.started, sync.completed, sync.conflict,
 *         user.created, user.deactivated,
 *         policy.violation, approval.requested, approval.resolved,
 *         connector.connected, connector.disconnected
 */

interface Webhook {
  id: string;
  url: string;
  events: string[];
  secret: string;
  is_active: boolean;
  created_at: string;
  updated_at: string;
  last_triggered: string | null;
  failure_count: number;
  created_by: string;
}

const webhooks: Webhook[] = [];

const VALID_EVENTS = [
  'transfer.started',
  'transfer.completed',
  'transfer.failed',
  'transfer.paused',
  'sync.started',
  'sync.completed',
  'sync.conflict',
  'sync.error',
  'user.created',
  'user.deactivated',
  'user.role_changed',
  'policy.violation',
  'policy.updated',
  'approval.requested',
  'approval.resolved',
  'connector.connected',
  'connector.disconnected',
  'connector.error',
  'ai.action_executed',
  'ai.suggestion_generated',
];

function getApiKey(request: NextRequest): string | null {
  const auth = request.headers.get('authorization');
  if (auth?.startsWith('Bearer ')) return auth.slice(7);
  return request.headers.get('x-api-key');
}

/** GET /api/webhooks */
export async function GET(request: NextRequest) {
  const apiKey = getApiKey(request);
  if (!apiKey) {
    return NextResponse.json({ error: 'Authentication required' }, { status: 401 });
  }

  return NextResponse.json({
    webhooks: webhooks.map(w => ({
      ...w,
      secret: w.secret.slice(0, 4) + '****', // Mask secret
    })),
    total: webhooks.length,
    available_events: VALID_EVENTS,
  });
}

/** POST /api/webhooks */
export async function POST(request: NextRequest) {
  const authResult = await requireRole(request, 'manager');
  if (!isAuthorized(authResult)) return authResult;

  try {
    const body = await request.json();
    const { url, events } = body;

    if (!url) {
      return NextResponse.json({ error: 'url is required' }, { status: 400 });
    }

    // Validate URL format
    try {
      new URL(url);
    } catch {
      return NextResponse.json({ error: 'Invalid URL format' }, { status: 400 });
    }

    if (!events || !Array.isArray(events) || events.length === 0) {
      return NextResponse.json(
        { error: 'events array is required (at least one event)' },
        { status: 400 }
      );
    }

    const invalidEvents = events.filter((e: string) => !VALID_EVENTS.includes(e));
    if (invalidEvents.length > 0) {
      return NextResponse.json(
        {
          error: `Invalid events: ${invalidEvents.join(', ')}`,
          valid_events: VALID_EVENTS,
        },
        { status: 400 }
      );
    }

    const secret = `whsec_${uuidv4().replace(/-/g, '')}`;

    const webhook: Webhook = {
      id: `wh_${uuidv4().slice(0, 12)}`,
      url,
      events,
      secret,
      is_active: true,
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
      last_triggered: null,
      failure_count: 0,
      created_by: authResult.user.email,
    };

    webhooks.push(webhook);

    return NextResponse.json(
      {
        webhook: {
          ...webhook,
          secret, // Show full secret only on creation
        },
      },
      { status: 201 }
    );
  } catch {
    return NextResponse.json({ error: 'Invalid request body' }, { status: 400 });
  }
}
