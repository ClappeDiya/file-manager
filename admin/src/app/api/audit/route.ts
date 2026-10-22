import { NextRequest, NextResponse } from 'next/server';
import type { AuditEntry, AuditExportFormat } from '@/lib/types/audit';
import { exportAuditEntries } from '@/lib/utils/audit-integrity';
import { isContext, requirePermission } from '@/lib/session';
import { listAudit } from '@/lib/db/repos/audit';
import { managedSubtree } from '@/lib/db/repos/orgs';
import { pageInfo } from '@/lib/pagination';
import { pagedJson, readPage } from '@/lib/route-helpers';

const SYNC_EXPORT_LIMIT = 10_000;

/** GET /api/audit — the current organization's (and managed sub-organizations') audit log. */
export async function GET(request: NextRequest) {
  const { searchParams } = new URL(request.url);
  const exportFormat = searchParams.get('export') as AuditExportFormat | null;
  const ctx = await requirePermission(request, 'audit.read', ...(exportFormat ? (['audit.export'] as const) : []));
  if (!isContext(ctx)) return ctx;
  const dateTo = searchParams.get('dateTo');
  const q = {
    orgIds: managedSubtree(ctx.orgId),
    eventType: searchParams.get('eventType') || undefined,
    severity: searchParams.get('severity') || undefined,
    userId: searchParams.get('userId') || undefined,
    from: searchParams.get('dateFrom') || undefined,
    to: dateTo ? (dateTo.length === 10 ? `${dateTo}T23:59:59.999Z` : dateTo) : undefined,
    search: searchParams.get('search') || undefined,
  };
  if (exportFormat) {
    if (!['csv', 'json', 'syslog'].includes(exportFormat)) return NextResponse.json({ error: 'Unknown export format' }, { status: 400 });
    // Larger exports run in the background (POST /api/exports) instead of being cut short.
    const { entries, total } = listAudit({ ...q, limit: SYNC_EXPORT_LIMIT });
    if (total > SYNC_EXPORT_LIMIT) return NextResponse.json({ error: `${total} entries match; exports over ${SYNC_EXPORT_LIMIT} run in the background (Export → Start export)`, code: 'use_background_export', total }, { status: 413 });
    const types = { csv: 'text/csv', json: 'application/json', syslog: 'text/plain' } as const;
    return new NextResponse(exportAuditEntries(entries as unknown as AuditEntry[], exportFormat), {
      headers: { 'Content-Type': types[exportFormat], 'Content-Disposition': `attachment; filename="audit-export.${exportFormat}"` },
    });
  }
  const pr = readPage(request, { maxLimit: 200 });
  if (pr instanceof NextResponse) return pr;
  const { entries, total } = listAudit({ ...q, limit: pr.limit, offset: pr.offset });
  // `page`/`pageSize`/`totalPages` stay for existing callers.
  return pagedJson(request, { entries, total, page: Math.floor(pr.offset / pr.limit) + 1, pageSize: pr.limit, totalPages: Math.ceil(total / pr.limit) }, pageInfo(pr, entries.length, total));
}

/** Audit entries are append-only and written by the console itself. */
export async function POST() {
  return NextResponse.json({ error: 'Audit entries are immutable and created by system events only' }, { status: 405 });
}
