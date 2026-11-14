import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission, requireStepUp, type AuthContext } from '@/lib/session';
import { getOrg } from '@/lib/db/repos/orgs';
import { lastAuditPrune, verifyAudit } from '@/lib/db/repos/audit';
import { applyRetention, getRetention, LOG_TYPES, setRetention } from '@/lib/retention';
import { orgErrorResponse } from '@/lib/route-helpers';

/** Retention is console-wide: owners of a top-level organization manage it. */
const consoleOwner = (ctx: AuthContext) => ctx.role === 'org_owner' && !getOrg(ctx.orgId)?.parentId;

/** GET /api/retention — settings, log types and the audit log's prune state. */
export async function GET(request: NextRequest) {
  const ctx = await requirePermission(request, 'audit.read');
  if (!isContext(ctx)) return ctx;
  const v = verifyAudit();
  return NextResponse.json({ settings: getRetention(), types: LOG_TYPES, canEdit: consoleOwner(ctx), lastAuditPrune: lastAuditPrune(), auditVerifies: v.ok, auditReason: v.reason });
}

/** PUT /api/retention {audit?: days, api_requests?: days, …, applyNow?: boolean} */
export async function PUT(request: NextRequest) {
  const ctx = await requireStepUp(request, 'audit.export');
  if (!isContext(ctx)) return ctx;
  if (!consoleOwner(ctx)) return badRequest('Only owners of a top-level organization can change retention (it applies to the whole console)', 403);
  const b = (await readJson<Record<string, unknown>>(request)) ?? {};
  const { applyNow, ...patch } = b;
  try {
    const settings = setRetention(patch, ctx.user.email);
    audit(ctx, {
      eventType: 'retention.update',
      severity: 'warning',
      description: `Changed log retention (${Object.entries(patch)
        .map(([k, v]) => `${k}=${v}`)
        .join(', ')})`,
      metadata: { settings },
    });
    const removed = applyNow ? applyRetention(new Date(), ctx.user.email) : {};
    if (applyNow)
      audit(ctx, {
        eventType: 'retention.apply',
        severity: 'warning',
        description: `Applied retention: ${
          Object.entries(removed)
            .map(([k, n]) => `${k} ${n}`)
            .join(', ') || 'nothing to remove'
        }`,
        metadata: { removed },
      });
    return NextResponse.json({ settings, removed });
  } catch (e) {
    if (e instanceof Error && /does not verify/.test(e.message)) return badRequest(e.message, 409);
    return orgErrorResponse(e);
  }
}
