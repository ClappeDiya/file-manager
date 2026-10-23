import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { publicOrigin } from '@/lib/route-helpers';
import { createInvitation } from '@/lib/db/repos/invitations';
import { OrgError } from '@/lib/db/repos/orgs';
import { sendInvitationEmail } from '@/lib/mailer';
import { planImport } from '@/lib/csv-import';

const MAX_BYTES = 1_000_000;

/**
 * POST /api/users/import {csv, dryRun?}
 * Dry run: what each row would do. Otherwise: invite every valid row (an
 * error on one row never stops the others) and report the outcome per row.
 */
export async function POST(request: NextRequest) {
  const ctx = await requirePermission(request, 'users.invite');
  if (!isContext(ctx)) return ctx;
  const b = await readJson<{ csv?: string; dryRun?: boolean }>(request);
  if (typeof b?.csv !== 'string' || !b.csv.trim()) return badRequest('Paste or upload a CSV file');
  if (b.csv.length > MAX_BYTES) return badRequest('The file is larger than 1 MB', 413);
  const plan = planImport(ctx.orgId, ctx.role, b.csv);
  if (plan.error) return badRequest(plan.error);
  const summary = () => ({ invited: plan.rows.filter((r) => r.action === 'invite').length, skipped: plan.rows.filter((r) => r.action === 'skip').length, errors: plan.rows.filter((r) => r.action === 'error').length });
  if (b.dryRun) return NextResponse.json({ dryRun: true, rows: plan.rows, summary: summary() });

  let emailed = 0;
  for (const row of plan.rows.filter((r) => r.action === 'invite')) {
    try {
      const { invitation, token } = createInvitation(ctx.orgId, { email: row.email, role: row.role, groupId: row.groupId, groupRole: row.groupRole }, ctx.user.email);
      const d = await sendInvitationEmail(ctx.orgId, invitation.email, `${publicOrigin(request)}/invite/${encodeURIComponent(token)}`, ctx.user.name);
      if (d.sent) emailed++;
      row.message = d.sent ? 'Invited (e-mail sent)' : 'Invited (no e-mail channel; share the link from the invitations list)';
    } catch (e) {
      row.action = 'error';
      row.message = e instanceof OrgError ? e.message : 'Could not invite';
    }
  }
  const s = summary();
  audit(ctx, { eventType: 'admin.user_import', description: `Imported users from CSV: ${s.invited} invited, ${s.skipped} skipped, ${s.errors} errors`, resourceType: 'organization', resourceId: ctx.orgId, metadata: { ...s, emailed, invited: plan.rows.filter((r) => r.action === 'invite').map((r) => r.email) } });
  return NextResponse.json({ dryRun: false, rows: plan.rows, summary: { ...s, emailed } });
}
