import { NextRequest, NextResponse } from 'next/server';
import { audit, badRequest, isContext, readJson, requirePermission } from '@/lib/session';
import { getMachine } from '@/lib/db/repos/machines';
import { cancelCommand, COMMAND_KINDS, enqueueCommand, getCommand, listCommands, type CommandKind } from '@/lib/db/repos/commands';
import { canSeeMachine } from '@/lib/scope';

type P = { params: Promise<{ id: string }> };

/**
 * Client API calls a remote session may make: job management, runs and
 * maintenance. Calls that read secrets back or touch arbitrary local files
 * (config export with secrets, remote file get/put, file encryption) are excluded.
 */
const RPC_ALLOW = new Set([
  'backup_list_jobs', 'backup_get_job', 'backup_save_job', 'backup_delete_job', 'backup_test_destination', 'backup_run', 'backup_execute',
  'backup_cancel', 'backup_status', 'backup_pause', 'backup_resume', 'backup_list_versions', 'backup_list_files', 'backup_search', 'backup_compare',
  'backup_run_history', 'backup_export_cli', 'backup_capabilities', 'backup_preview_sources', 'backup_effective_job', 'backup_list_destinations',
  'backup_database_inventory', 'backup_diagnose', 'backup_job_placeholders', 'backup_test_secret_providers', 'backup_start_operation',
  'backup_cloud_providers', 'backup_remote_list', 'backup_get_global_defaults', 'backup_set_global_defaults', 'backup_test_notification',
]);

/** GET /api/fleet/machines/:id/commands[?commandId=] */
export async function GET(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'remote.execute');
  if (!isContext(ctx)) return ctx;
  const m = getMachine((await params).id);
  if (!canSeeMachine(ctx, m)) return badRequest('Machine not found', 404);
  const one = new URL(request.url).searchParams.get('commandId');
  if (one) {
    const c = getCommand(one);
    return c && c.machineId === m.id ? NextResponse.json({ command: c }) : badRequest('Command not found', 404);
  }
  return NextResponse.json({ commands: listCommands(ctx.orgId, m.id) });
}

/** POST /api/fleet/machines/:id/commands {kind, payload} — queue a remote command. */
export async function POST(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'remote.execute');
  if (!isContext(ctx)) return ctx;
  const m = getMachine((await params).id);
  if (!canSeeMachine(ctx, m)) return badRequest('Machine not found', 404);
  if (m.status !== 'active') return badRequest('The machine is not active', 409);
  const b = await readJson<{ kind?: CommandKind; payload?: Record<string, unknown> }>(request);
  if (!b?.kind || !COMMAND_KINDS.includes(b.kind)) return badRequest('Unknown command');
  const payload = b.payload ?? {};
  if (b.kind === 'rpc' && (typeof payload.command !== 'string' || !RPC_ALLOW.has(payload.command))) return badRequest('This client command is not allowed remotely', 403);
  if (b.kind === 'run_backup' && typeof payload.jobId !== 'string') return badRequest('jobId is required');
  const cmd = enqueueCommand(ctx.orgId, m.id, b.kind, payload, ctx.user.email);
  audit(ctx, { eventType: 'remote.command', description: `Sent ${b.kind}${b.kind === 'rpc' ? ` ${payload.command}` : ''} to "${m.name}"`, resourceType: 'machine', resourceId: m.id, metadata: { commandId: cmd.id } });
  return NextResponse.json({ command: cmd }, { status: 201 });
}

/** DELETE /api/fleet/machines/:id/commands?commandId= — cancel a queued command. */
export async function DELETE(request: NextRequest, { params }: P) {
  const ctx = await requirePermission(request, 'remote.execute');
  if (!isContext(ctx)) return ctx;
  const m = getMachine((await params).id);
  if (!canSeeMachine(ctx, m)) return badRequest('Machine not found', 404);
  const id = new URL(request.url).searchParams.get('commandId') ?? '';
  if (!cancelCommand(ctx.orgId, id)) return badRequest('No queued command with this id', 404);
  return NextResponse.json({ success: true });
}
