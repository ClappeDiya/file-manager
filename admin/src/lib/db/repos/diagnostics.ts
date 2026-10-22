/**
 * Diagnostics bundles (zip: scrubbed logs, versions, OS) uploaded by machine
 * agents, on demand or for a `collect_diagnostics` command. Only the newest
 * few per machine are kept.
 */
import { createHash } from 'node:crypto';
import { db, newId, now, parseJson, type Row } from '../index';

/** Largest bundle accepted (the agent refuses bigger ones before upload). */
export const MAX_DIAGNOSTICS_BYTES = 8 * 1024 * 1024;
export const KEEP_PER_MACHINE = 10;

export interface Diagnostics {
  id: string;
  machineId: string;
  fileName: string;
  size: number;
  sha256: string;
  summary: Record<string, unknown>;
  createdAt: string;
}

const COLS = 'id, machine_id, file_name, size, sha256, summary, created_at';

function fromRow(r: Row): Diagnostics {
  return {
    id: r.id as string,
    machineId: r.machine_id as string,
    fileName: r.file_name as string,
    size: r.size as number,
    sha256: r.sha256 as string,
    summary: parseJson(r.summary, {}),
    createdAt: r.created_at as string,
  };
}

export class DiagnosticsError extends Error {
  constructor(message: string, readonly status = 422) {
    super(message);
  }
}

/** Validate an agent upload `{fileName, summary, bundleBase64}` into bytes. */
export function decodeUpload(raw: Record<string, unknown>): { fileName: string; summary: Record<string, unknown>; data: Buffer } {
  const b64 = typeof raw.bundleBase64 === 'string' ? raw.bundleBase64 : '';
  if (!b64) throw new DiagnosticsError('bundleBase64 is required', 400);
  if (b64.length > Math.ceil(MAX_DIAGNOSTICS_BYTES / 3) * 4 + 4) throw new DiagnosticsError('The diagnostics bundle is too large', 413);
  const data = Buffer.from(b64, 'base64');
  if (data.length > MAX_DIAGNOSTICS_BYTES) throw new DiagnosticsError('The diagnostics bundle is too large', 413);
  if (data.length < 4 || data[0] !== 0x50 || data[1] !== 0x4b) throw new DiagnosticsError('The diagnostics bundle is not a zip file');
  const name = (typeof raw.fileName === 'string' ? raw.fileName : '').replace(/[^A-Za-z0-9._-]/g, '_').slice(0, 120);
  const summary = raw.summary && typeof raw.summary === 'object' && !Array.isArray(raw.summary) ? (raw.summary as Record<string, unknown>) : {};
  const json = JSON.stringify(summary);
  return { fileName: name.endsWith('.zip') ? name : `${name || 'diagnostics'}.zip`, summary: json.length > 20_000 ? { truncated: true } : summary, data };
}

export function storeDiagnostics(orgId: string, machineId: string, u: { fileName: string; summary: Record<string, unknown>; data: Buffer }): Diagnostics {
  const id = newId('dgn');
  const d = db();
  d.prepare('INSERT INTO machine_diagnostics (id, org_id, machine_id, file_name, size, sha256, summary, data, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)').run(
    id,
    orgId,
    machineId,
    u.fileName,
    u.data.length,
    createHash('sha256').update(u.data).digest('hex'),
    JSON.stringify(u.summary),
    u.data,
    now(),
  );
  d.prepare('DELETE FROM machine_diagnostics WHERE machine_id = ? AND id NOT IN (SELECT id FROM machine_diagnostics WHERE machine_id = ? ORDER BY created_at DESC, rowid DESC LIMIT ?)').run(machineId, machineId, KEEP_PER_MACHINE);
  return getDiagnostics(machineId, id)!;
}

export function listDiagnostics(machineId: string): Diagnostics[] {
  return (db().prepare(`SELECT ${COLS} FROM machine_diagnostics WHERE machine_id = ? ORDER BY created_at DESC, rowid DESC`).all(machineId) as Row[]).map(fromRow);
}

export function getDiagnostics(machineId: string, id: string): Diagnostics | null {
  const r = db().prepare(`SELECT ${COLS} FROM machine_diagnostics WHERE machine_id = ? AND id = ?`).get(machineId, id) as Row | undefined;
  return r ? fromRow(r) : null;
}

export function diagnosticsData(machineId: string, id: string): Buffer | null {
  const r = db().prepare('SELECT data FROM machine_diagnostics WHERE machine_id = ? AND id = ?').get(machineId, id) as { data: Buffer } | undefined;
  return r?.data ?? null;
}

export function deleteDiagnostics(machineId: string, id: string): boolean {
  return db().prepare('DELETE FROM machine_diagnostics WHERE machine_id = ? AND id = ?').run(machineId, id).changes > 0;
}
