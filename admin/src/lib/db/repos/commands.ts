/**
 * Commands queued for machines (remote sessions). The agent fetches queued
 * commands on its heartbeat, runs them and posts the result back.
 *
 * Kinds:
 * - `rpc`            {command, args} — any client API call (same surface as the
 *                    client's own REST API: jobs, runs, restore, maintenance…)
 * - `run_backup`     {jobId}
 * - `apply_policies` {} — re-fetch and apply fleet policies now
 * - `rotate_token`   {} — the agent swaps to a new token
 * - `collect_diagnostics` {} — the agent uploads a diagnostics bundle
 */
import { db, newId, now, parseJson, type Row } from '../index';

export type CommandKind = 'rpc' | 'run_backup' | 'apply_policies' | 'rotate_token' | 'collect_diagnostics';
export type CommandStatus = 'queued' | 'delivered' | 'succeeded' | 'failed' | 'expired' | 'cancelled';
export const COMMAND_KINDS: CommandKind[] = ['rpc', 'run_backup', 'apply_policies', 'rotate_token', 'collect_diagnostics'];

export interface AgentCommand {
  id: string;
  orgId: string;
  machineId: string;
  kind: CommandKind;
  payload: Record<string, unknown>;
  status: CommandStatus;
  result: unknown;
  createdBy: string;
  createdAt: string;
  deliveredAt: string | null;
  completedAt: string | null;
  expiresAt: string;
}

function fromRow(r: Row): AgentCommand {
  const expired = (r.status === 'queued' || r.status === 'delivered') && new Date(r.expires_at as string) <= new Date();
  return {
    id: r.id as string,
    orgId: r.org_id as string,
    machineId: r.machine_id as string,
    kind: r.kind as CommandKind,
    payload: parseJson(r.payload, {}),
    status: (expired ? 'expired' : r.status) as CommandStatus,
    result: parseJson(r.result, null),
    createdBy: r.created_by as string,
    createdAt: r.created_at as string,
    deliveredAt: (r.delivered_at as string) ?? null,
    completedAt: (r.completed_at as string) ?? null,
    expiresAt: r.expires_at as string,
  };
}

export function enqueueCommand(orgId: string, machineId: string, kind: CommandKind, payload: Record<string, unknown>, by: string, ttlMinutes = 60): AgentCommand {
  const id = newId('cmd');
  db()
    .prepare('INSERT INTO agent_commands (id, org_id, machine_id, kind, payload, created_by, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)')
    .run(id, orgId, machineId, kind, JSON.stringify(payload), by, now(), new Date(Date.now() + ttlMinutes * 60_000).toISOString());
  return getCommand(id)!;
}

export function getCommand(id: string): AgentCommand | null {
  const r = db().prepare('SELECT * FROM agent_commands WHERE id = ?').get(id) as Row | undefined;
  return r ? fromRow(r) : null;
}

/** Hand queued commands to the agent (oldest first) and mark them delivered. */
export function takeCommands(machineId: string, max = 10): AgentCommand[] {
  const rows = db().prepare("SELECT * FROM agent_commands WHERE machine_id = ? AND status = 'queued' AND expires_at > ? ORDER BY created_at LIMIT ?").all(machineId, now(), max) as Row[];
  const mark = db().prepare("UPDATE agent_commands SET status = 'delivered', delivered_at = ? WHERE id = ?");
  for (const r of rows) mark.run(now(), r.id);
  return rows.map((r) => ({ ...fromRow(r), status: 'delivered' as const }));
}

export function completeCommand(machineId: string, id: string, ok: boolean, result: unknown): boolean {
  const body = JSON.stringify(result ?? null);
  return db()
    .prepare("UPDATE agent_commands SET status = ?, result = ?, completed_at = ? WHERE id = ? AND machine_id = ? AND status IN ('queued', 'delivered')")
    .run(ok ? 'succeeded' : 'failed', body.length > 1_000_000 ? JSON.stringify({ truncated: true, preview: body.slice(0, 100_000) }) : body, now(), id, machineId).changes > 0;
}

export function cancelCommand(orgId: string, id: string): boolean {
  return db().prepare("UPDATE agent_commands SET status = 'cancelled', completed_at = ? WHERE org_id = ? AND id = ? AND status = 'queued'").run(now(), orgId, id).changes > 0;
}

export function listCommands(orgId: string, machineId: string, limit = 50): AgentCommand[] {
  return (db().prepare('SELECT * FROM agent_commands WHERE org_id = ? AND machine_id = ? ORDER BY created_at DESC LIMIT ?').all(orgId, machineId, limit) as Row[]).map(fromRow);
}
