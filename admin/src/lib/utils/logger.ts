/**
 * Structured logging utility for admin API route handlers.
 *
 * Outputs JSON-formatted log lines with consistent fields for
 * observability tooling (log drains, Coolify logs, Grafana, etc.).
 *
 * Usage in route handlers:
 *   import { logger } from '@/lib/utils/logger';
 *   logger.info('user.login', { email, ip });
 *   logger.warn('rate.limited', { ip, retryAfter });
 *   logger.error('auth.failed', { reason: 'invalid_password', email });
 */

type LogLevel = 'info' | 'warn' | 'error';

interface LogEntry {
  level: LogLevel;
  event: string;
  timestamp: string;
  [key: string]: unknown;
}

function emit(level: LogLevel, event: string, data?: Record<string, unknown>): void {
  const entry: LogEntry = {
    level,
    event,
    timestamp: new Date().toISOString(),
    ...data,
  };

  const line = JSON.stringify(entry);

  switch (level) {
    case 'error':
      console.error(line);
      break;
    case 'warn':
      console.warn(line);
      break;
    default:
      console.log(line);
  }
}

export const logger = {
  info: (event: string, data?: Record<string, unknown>) => emit('info', event, data),
  warn: (event: string, data?: Record<string, unknown>) => emit('warn', event, data),
  error: (event: string, data?: Record<string, unknown>) => emit('error', event, data),
};
