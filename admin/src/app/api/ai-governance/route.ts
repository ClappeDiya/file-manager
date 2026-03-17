import { NextRequest, NextResponse } from 'next/server';

/**
 * AI Governance API (T-059)
 *
 * GET /api/ai-governance - Get AI governance settings and audit log
 * POST /api/ai-governance - Update AI governance settings
 * GET /api/ai-governance?view=audit - Get AI action audit log
 * GET /api/ai-governance?view=toggles - Get feature toggles
 */

interface AiGovernanceSettings {
  ai_enabled: boolean;
  auto_suggestions: boolean;
  natural_language_commands: boolean;
  require_confirmation_for_destructive: boolean;
  max_files_per_action: number;
  allowed_actions: string[];
  audit_retention_days: number;
  model_provider: string;
}

interface AiAuditEntry {
  id: string;
  action: string;
  user: string;
  input: string;
  output: string;
  approved: boolean;
  timestamp: string;
  files_affected: number;
}

const settings: AiGovernanceSettings = {
  ai_enabled: true,
  auto_suggestions: true,
  natural_language_commands: true,
  require_confirmation_for_destructive: true,
  max_files_per_action: 100,
  allowed_actions: ['organize', 'rename', 'tag', 'move', 'copy'],
  audit_retention_days: 90,
  model_provider: 'local',
};

const auditLog: AiAuditEntry[] = [
  {
    id: 'ai_001',
    action: 'organize',
    user: 'admin@ufop.io',
    input: 'Organize downloads folder by type',
    output: 'Moved 45 files into 6 subfolders',
    approved: true,
    timestamp: new Date(Date.now() - 3600000).toISOString(),
    files_affected: 45,
  },
];

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

  if (view === 'audit') {
    const limit = parseInt(searchParams.get('limit') || '50');
    const offset = parseInt(searchParams.get('offset') || '0');
    return NextResponse.json({
      entries: auditLog.slice(offset, offset + limit),
      total: auditLog.length,
    });
  }

  if (view === 'toggles') {
    return NextResponse.json({
      toggles: {
        ai_enabled: settings.ai_enabled,
        auto_suggestions: settings.auto_suggestions,
        natural_language_commands: settings.natural_language_commands,
        require_confirmation_for_destructive: settings.require_confirmation_for_destructive,
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

    // Validate and merge settings
    if (body.ai_enabled !== undefined) settings.ai_enabled = Boolean(body.ai_enabled);
    if (body.auto_suggestions !== undefined) settings.auto_suggestions = Boolean(body.auto_suggestions);
    if (body.natural_language_commands !== undefined) settings.natural_language_commands = Boolean(body.natural_language_commands);
    if (body.require_confirmation_for_destructive !== undefined)
      settings.require_confirmation_for_destructive = Boolean(body.require_confirmation_for_destructive);
    if (body.max_files_per_action !== undefined) settings.max_files_per_action = Number(body.max_files_per_action);
    if (body.allowed_actions) settings.allowed_actions = body.allowed_actions;
    if (body.audit_retention_days !== undefined) settings.audit_retention_days = Number(body.audit_retention_days);
    if (body.model_provider) settings.model_provider = body.model_provider;

    // Audit this settings change
    auditLog.push({
      id: `ai_${Date.now()}`,
      action: 'settings_updated',
      user: apiKey.slice(0, 8),
      input: JSON.stringify(body),
      output: 'AI governance settings updated',
      approved: true,
      timestamp: new Date().toISOString(),
      files_affected: 0,
    });

    return NextResponse.json({ settings });
  } catch {
    return NextResponse.json({ error: 'Invalid request body' }, { status: 400 });
  }
}
