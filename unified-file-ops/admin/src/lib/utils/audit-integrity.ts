/**
 * Audit log integrity utilities (T-052).
 * Provides hash chain verification for immutable audit entries.
 */

import { type AuditEntry, type AuditExportFormat } from '../types/audit';

/**
 * Create a hash of an audit entry for integrity verification.
 * In production this would use a server-side crypto module.
 */
export async function computeEntryHash(entry: Omit<AuditEntry, 'integrityHash'>): Promise<string> {
  const content = JSON.stringify({
    id: entry.id,
    eventType: entry.eventType,
    severity: entry.severity,
    timestamp: entry.timestamp,
    userId: entry.userId,
    organizationId: entry.organizationId,
    description: entry.description,
    metadata: entry.metadata,
    previousHash: entry.previousHash,
  });

  const encoder = new TextEncoder();
  const data = encoder.encode(content);
  const hashBuffer = await crypto.subtle.digest('SHA-256', data);
  const hashArray = Array.from(new Uint8Array(hashBuffer));
  return hashArray.map(b => b.toString(16).padStart(2, '0')).join('');
}

/**
 * Verify the integrity chain of audit entries.
 */
export async function verifyAuditChain(
  entries: AuditEntry[]
): Promise<{ valid: boolean; brokenAt?: number; details: string }> {
  if (entries.length === 0) {
    return { valid: true, details: 'No entries to verify' };
  }

  // Sort by timestamp
  const sorted = [...entries].sort(
    (a, b) => new Date(a.timestamp).getTime() - new Date(b.timestamp).getTime()
  );

  for (let i = 0; i < sorted.length; i++) {
    const entry = sorted[i];

    // Verify hash of current entry
    const computedHash = await computeEntryHash({
      ...entry,
    });

    if (computedHash !== entry.integrityHash) {
      return {
        valid: false,
        brokenAt: i,
        details: `Entry ${entry.id} has been tampered with: hash mismatch`,
      };
    }

    // Verify chain link (if not the first entry)
    if (i > 0 && entry.previousHash !== sorted[i - 1].integrityHash) {
      return {
        valid: false,
        brokenAt: i,
        details: `Chain broken at entry ${entry.id}: previous hash mismatch`,
      };
    }
  }

  return { valid: true, details: `${sorted.length} entries verified successfully` };
}

/**
 * Export audit entries to the specified format.
 */
export function exportAuditEntries(
  entries: AuditEntry[],
  format: AuditExportFormat
): string {
  switch (format) {
    case 'json':
      return JSON.stringify(entries, null, 2);

    case 'csv': {
      const headers = [
        'ID', 'Event Type', 'Severity', 'Timestamp', 'User ID', 'User Name',
        'User Email', 'IP Address', 'Description', 'Resource Type', 'Resource ID',
        'Integrity Hash',
      ];
      const rows = entries.map(e => [
        e.id,
        e.eventType,
        e.severity,
        e.timestamp,
        e.userId,
        e.userName,
        e.userEmail,
        e.ipAddress,
        `"${e.description.replace(/"/g, '""')}"`,
        e.resourceType ?? '',
        e.resourceId ?? '',
        e.integrityHash,
      ]);
      return [headers.join(','), ...rows.map(r => r.join(','))].join('\n');
    }

    case 'syslog': {
      return entries
        .map(e => {
          const severity = { info: 6, warning: 4, error: 3, critical: 2 }[e.severity];
          return `<${severity}>1 ${e.timestamp} ufop-admin - ${e.eventType} - ${e.description} [meta userId="${e.userId}" ip="${e.ipAddress}"]`;
        })
        .join('\n');
    }

    default:
      throw new Error(`Unsupported export format: ${format}`);
  }
}
