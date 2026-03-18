import { NextRequest, NextResponse } from 'next/server';
import * as db from '@/lib/db';

/**
 * AI Governance API (T-059)
 *
 * GET /api/ai-governance - Get AI governance settings and audit log
 * POST /api/ai-governance - Update AI governance settings
 */

function getApiKey(request: NextRequest): string | null {
  const auth = request.headers.get('authorization');
  if (auth?.startsWith('Bearer ')) return auth.slice(7);
  return request.headers.get('x-api-key');
}

/** GET /api/ai-governance */
export async function GET(request: NextRequest) {
  const apiKey = getApiKey(request);
  if (!apiKey) {
    return NextResponse.json({ error: 'Authentication required' }, { status: 401 });
  }

  const { searchParams } = new URL(request.url);
  const view = searchParams.get('view');

  const settings = db.getAiGovernanceSettings();

  if (view === 'toggles') {
    return NextResponse.json({
      toggles: {
        fileClassification: settings.fileClassification ?? true,
        contentAnalysis: settings.contentAnalysis ?? false,
        namingSuggestions: settings.namingSuggestions ?? true,
        anomalyDetection: settings.anomalyDetection ?? true,
      },
    });
  }

  return NextResponse.json({ settings });
}

/** POST /api/ai-governance - Update settings */
export async function POST(request: NextRequest) {
  const apiKey = getApiKey(request);
  if (!apiKey) {
    return NextResponse.json({ error: 'Authentication required' }, { status: 401 });
  }

  try {
    const body = await request.json();

    for (const [key, value] of Object.entries(body)) {
      db.setAiGovernanceSetting(key, value);
    }

    return NextResponse.json({ settings: db.getAiGovernanceSettings() });
  } catch {
    return NextResponse.json({ error: 'Invalid request body' }, { status: 400 });
  }
}
