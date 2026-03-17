import { NextRequest, NextResponse } from 'next/server';
import { dataStore } from '@/lib/data-store';
import type { AuditEventType, AuditSeverity, AuditExportFormat } from '@/lib/types/audit';
import { exportAuditEntries } from '@/lib/utils/audit-integrity';

/** GET /api/audit - Query audit log with search/filter */
export async function GET(request: NextRequest) {
  const { searchParams } = new URL(request.url);
  const eventType = searchParams.get('eventType') as AuditEventType | null;
  const severity = searchParams.get('severity') as AuditSeverity | null;
  const userId = searchParams.get('userId');
  const dateFrom = searchParams.get('dateFrom');
  const dateTo = searchParams.get('dateTo');
  const search = searchParams.get('search');
  const exportFormat = searchParams.get('export') as AuditExportFormat | null;
  const page = parseInt(searchParams.get('page') || '1');
  const pageSize = parseInt(searchParams.get('pageSize') || '50');

  let entries = [...dataStore.auditEntries];

  if (eventType) entries = entries.filter(e => e.eventType === eventType);
  if (severity) entries = entries.filter(e => e.severity === severity);
  if (userId) entries = entries.filter(e => e.userId === userId);
  if (dateFrom) entries = entries.filter(e => new Date(e.timestamp) >= new Date(dateFrom));
  if (dateTo) entries = entries.filter(e => new Date(e.timestamp) <= new Date(dateTo + 'T23:59:59Z'));
  if (search) {
    const q = search.toLowerCase();
    entries = entries.filter(e =>
      e.description.toLowerCase().includes(q) ||
      e.userName.toLowerCase().includes(q) ||
      e.eventType.toLowerCase().includes(q)
    );
  }

  // Sort by timestamp descending
  entries.sort((a, b) => new Date(b.timestamp).getTime() - new Date(a.timestamp).getTime());

  // Export if requested
  if (exportFormat) {
    const content = exportAuditEntries(entries, exportFormat);
    const contentTypes = {
      csv: 'text/csv',
      json: 'application/json',
      syslog: 'text/plain',
    };

    return new NextResponse(content, {
      headers: {
        'Content-Type': contentTypes[exportFormat] || 'text/plain',
        'Content-Disposition': `attachment; filename="audit-export.${exportFormat}"`,
      },
    });
  }

  // Paginate
  const total = entries.length;
  const start = (page - 1) * pageSize;
  const paged = entries.slice(start, start + pageSize);

  return NextResponse.json({
    entries: paged,
    total,
    page,
    pageSize,
    totalPages: Math.ceil(total / pageSize),
  });
}

/**
 * Audit entries are immutable - no POST/PUT/DELETE allowed.
 * Entries are created internally by system events, not via API.
 */
export async function POST() {
  return NextResponse.json(
    { error: 'Audit entries are immutable and created by system events only' },
    { status: 405 }
  );
}
