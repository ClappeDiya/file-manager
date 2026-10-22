/**
 * Admin console database: one SQLite file (ADMIN_DB_PATH, default
 * ./admin-data.db) with versioned migrations (`PRAGMA user_version`).
 *
 * Server-only. Repositories in ./repos take the handle from `db()`; tests set
 * ADMIN_DB_PATH=':memory:' and call `resetDb()` between cases.
 */
import Database from 'better-sqlite3';
import crypto from 'crypto';
import path from 'path';
import { MIGRATIONS } from './migrations';

let handle: Database.Database | null = null;

function open(): Database.Database {
  const file = process.env.ADMIN_DB_PATH || path.join(process.cwd(), 'admin-data.db');
  const d = new Database(file);
  d.pragma('journal_mode = WAL');
  d.pragma('foreign_keys = ON');
  d.pragma('busy_timeout = 5000');
  migrate(d);
  return d;
}

/** Apply every migration newer than the database's `user_version`. */
export function migrate(d: Database.Database): number {
  const current = d.pragma('user_version', { simple: true }) as number;
  let applied = 0;
  for (const m of MIGRATIONS) {
    if (m.version <= current) continue;
    d.transaction(() => {
      m.up(d);
      d.pragma(`user_version = ${m.version}`);
    })();
    applied++;
  }
  return applied;
}

export function schemaVersion(): number {
  return db().pragma('user_version', { simple: true }) as number;
}

export function db(): Database.Database {
  if (!handle) handle = open();
  return handle;
}

/** Close and forget the handle (tests: a fresh in-memory database). */
export function resetDb(): void {
  handle?.close();
  handle = null;
}

export function tx<T>(fn: () => T): T {
  return db().transaction(fn)();
}

export const now = () => new Date().toISOString();
export const newId = (prefix: string) => `${prefix}_${crypto.randomUUID().replace(/-/g, '').slice(0, 20)}`;

export function parseJson<T>(raw: unknown, fallback: T): T {
  if (typeof raw !== 'string' || raw === '') return fallback;
  try {
    return JSON.parse(raw) as T;
  } catch {
    return fallback;
  }
}

/** SHA-256 hex of a secret token (tokens are stored hashed). */
export function hashToken(token: string): string {
  return crypto.createHash('sha256').update(token).digest('hex');
}

/** A random URL-safe token with a readable prefix (`ufop_reg_…`). */
export function newToken(prefix: string, bytes = 24): string {
  return `${prefix}_${crypto.randomBytes(bytes).toString('base64url')}`;
}

export type Row = Record<string, unknown>;
