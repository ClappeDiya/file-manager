import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, can, canIn, isContext, readJson, requirePermission } from '@/lib/session';
import { deleteMachine, getMachine, setMachineStatus, updateMachine } from '@/lib/db/repos/machines';
import { listBackups, listReports } from '@/lib/db/repos/reports';
import { listAlerts } from '@/lib/db/repos/alerts';
import { listCommands, enqueueCommand } from '@/lib/db/repos/commands';
import { policiesFor } from '@/lib/db/repos/fleet-policies';
import { OrgError } from '@/lib/db/repos/orgs';
import { canSeeMachine } from '@/lib/scope';

type P = { params: Promise<{ id: string }> };

/** GET /api/fleet/machines/:id — details, backups, recent reports, alerts, commands, policies. */
export async function GET(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'fleet.read');
  if (!isContext(ctx)) return ctx;
  const m = getMachine((await params).id);
  if (!canSeeMachine(ctx, m)) return badRequest('Machine not found', 404);
  const policies = policiesFor(m).map((p) => ({ id: p.id, name: p.name, version: p.version, applied: m.appliedPolicies[p.id] ?? null }));
  return NextResponse.json({
    machine: m,
    backups: listBackups([ctx.orgId], [m.id]),
    reports: listReports({ orgIds: [ctx.orgId], machineId: m.id, limit: 50 }),
    alerts: listAlerts({ orgIds: [ctx.orgId], machineIds: [m.id], limit: 50 }),
    commands: can(ctx, 'remote.execute') ? listCommands(ctx.orgId, m.id) : [],
    policies,
  });
}

/** PATCH /api/fleet/machines/:id {name?, tags?, notes?, orgId?} or {action: suspend|resume|revoke|rotate_token} */
export async function PATCH(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'fleet.write');
  if (!isContext(ctx)) return ctx;
  const m = getMachine((await params).id);
  if (!canSeeMachine(ctx, m)) return badRequest('Machine not found', 404);
  const b = await readJson<{ name?: string; tags?: string[]; notes?: string; orgId?: string; action?: string }>(request);
  if (!b) return badRequest('Invalid request');
  if (b.action) {
    if (b.action === 'suspend' || b.action === 'resume') setMachineStatus(m.id, b.action === 'suspend' ? 'suspended' : 'active');
    else if (b.action === 'revoke') setMachineStatus(m.id, 'revoked');
    else if (b.action === 'rotate_token') {
      // The agent rotates itself on its next heartbeat; an admin-side rotation would strand it.
      enqueueCommand(ctx.orgId, m.id, 'rotate_token', {}, ctx.user.email, 24 * 60);
    } else return badRequest('Unknown action');
    audit(ctx, { eventType: `machine.${b.action}`, severity: b.action === 'revoke' ? 'warning' : 'info', description: `${b.action} "${m.name}"`, resourceType: 'machine', resourceId: m.id });
    return NextResponse.json({ machine: getMachine(m.id) });
  }
  if (b.orgId && b.orgId !== m.orgId && !canIn(ctx, b.orgId, 'fleet.write')) return badRequest('You cannot move machines to that organization', 403);
  try {
    const upd = updateMachine(m.id, b);
    audit(ctx, { eventType: 'machine.update', description: `Updated "${upd!.name}"`, resourceType: 'machine', resourceId: m.id, metadata: { fields: Object.keys(b) } });
    return NextResponse.json({ machine: upd });
  } catch (e) {
    if (e instanceof OrgError) return badRequest(e.message, e.status);
    throw e;
  }
}

/** DELETE /api/fleet/machines/:id — remove the machine and its history. */
export async function DELETE(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'fleet.write');
  if (!isContext(ctx)) return ctx;
  const m = getMachine((await params).id);
  if (!canSeeMachine(ctx, m)) return badRequest('Machine not found', 404);
  deleteMachine(m.id);
  audit(ctx, { eventType: 'machine.delete', severity: 'warning', description: `Removed "${m.name}" and its history`, resourceType: 'machine', resourceId: m.id });
  return NextResponse.json({ success: true });
}

