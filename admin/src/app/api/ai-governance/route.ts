import { NextRequest, NextResponse } from 'next/server';
import { pageInfo } from '@/lib/pagination';
import { pagedJson, readPage } from '@/lib/route-helpers';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { listAudit } from '@/lib/db/repos/audit';
import { getAiGovernance, GovernanceError, saveAiGovernance, validateAiGovernance } from '@/lib/db/repos/ai-governance';

/**
 * AI Governance API, scoped to the current organization.
 *
 * GET  /api/ai-governance              settings
 * GET  /api/ai-governance?view=toggles feature switches only
 * GET  /api/ai-governance?view=audit   AI-related audit entries (hash-chained log)
 * POST /api/ai-governance              partial update
 *
 * Settings used to live in module memory (lost on restart, shared by every
 * organization) and GET accepted any bearer token without checking it.
 */

/** Legacy snake_case keys the first version of this API accepted. */
function fromLegacy(body: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = { ...body };
  const rename: Record<string, string> = {
    ai_enabled: 'aiEnabled',
    require_confirmation_for_destructive: 'requireConfirmationForDestructive',
    max_files_per_action: 'maxFilesPerAction',
    allowed_actions: 'allowedActions',
    audit_retention_days: 'auditRetentionDays',
  };
  for (const [from, to] of Object.entries(rename)) {
    if (from in out) {
      out[to] = out[from];
      delete out[from];
    }
  }
  for (const f of ['auto_suggestions', 'natural_language_commands']) {
    if (f in out) {
      out.features = { ...((out.features as Record<string, unknown>) ?? {}), [f]: out[f] };
      delete out[f];
    }
  }
  delete out.model_provider;
  return out;
}

export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'ai_governance.read');
  if (!isContext(ctx)) return ctx;
  const view = new URL(request.url).searchParams.get('view');
  const settings = getAiGovernance(ctx.orgId);

  if (view === 'audit') {
    const pr = readPage(request);
    if (pr instanceof NextResponse) return pr;
    const { entries, total } = listAudit({ orgIds: [ctx.orgId], eventPrefix: 'ai.', limit: pr.limit, offset: pr.offset });
    return pagedJson(request, { entries, total }, pageInfo(pr, entries.length, total));
  }

  if (view === 'toggles') {
    return NextResponse.json({ toggles: { aiEnabled: settings.aiEnabled, ...settings.features } });
  }

  return NextResponse.json({ settings });
}

export async function POST(request: NextRequest) {
  const ctx = await requirePermission(request, 'ai_governance.write');
  if (!isContext(ctx)) return ctx;
  const body = await readJson(request);
  if (!body || typeof body !== 'object' || Array.isArray(body)) return badRequest('Invalid request body');

  const current = getAiGovernance(ctx.orgId);
  let next;
  try {
    next = validateAiGovernance(fromLegacy(body), current);
  } catch (e) {
    if (e instanceof GovernanceError) return badRequest(e.message);
    throw e;
  }
  saveAiGovernance(ctx.orgId, next, ctx.user.email);

  const changed = Object.keys(next).filter(
    (k) => JSON.stringify(next[k as keyof typeof next]) !== JSON.stringify(current[k as keyof typeof current]),
  );
  audit(ctx, {
    eventType: 'ai.governance.updated',
    severity: changed.includes('aiEnabled') || changed.includes('dataResidency') ? 'warning' : 'info',
    description: changed.length ? `AI governance updated: ${changed.join(', ')}` : 'AI governance saved (no changes)',
    resourceType: 'ai_governance',
    resourceId: ctx.orgId,
    metadata: { changed, before: Object.fromEntries(changed.map((k) => [k, current[k as keyof typeof current]])), after: Object.fromEntries(changed.map((k) => [k, next[k as keyof typeof next]])) },
  });
  return NextResponse.json({ settings: getAiGovernance(ctx.orgId) });
}
