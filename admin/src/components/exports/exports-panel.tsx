'use client';

/** Background exports: start one, watch its progress, download it when ready. */
import React, { useState } from 'react';
import { Button } from '@/components/ui/button';
import { ErrorBanner } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { formatBytes, formatRelativeTime } from '@/lib/utils/format';
import type { ExportFormat, ExportJob, ExportKind } from '@/lib/exports';

interface Props {
  kind: ExportKind;
  formats: ExportFormat[];
  /** Filters passed to the export (e.g. the audit page's current search). */
  params?: Record<string, string>;
}

const LABEL: Record<ExportFormat, string> = { csv: 'CSV', json: 'JSON', syslog: 'Syslog' };

export function ExportsPanel({ kind, formats, params = {} }: Props) {
  const { data, reload } = useApi<{ exports: ExportJob[] }>('/api/exports', 2_000);
  const [err, setErr] = useState<string | null>(null);
  const mine = (data?.exports ?? []).filter((e) => e.kind === kind);
  const start = async (format: ExportFormat) => {
    setErr(null);
    try {
      await api('/api/exports', { body: { kind, format, params } });
      await reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };
  return (
    <div className="space-y-2 text-sm">
      <ErrorBanner message={err} />
      <div className="flex flex-wrap gap-2">
        {formats.map((f) => (
          <Button key={f} size="sm" variant="outline" onClick={() => void start(f)}>
            Start {LABEL[f]} export
          </Button>
        ))}
      </div>
      <p className="text-xs text-foreground-tertiary">Exports include every matching row and run in the background; files are kept for 7 days.</p>
      <ul className="divide-y divide-border">
        {mine.map((e) => (
          <li key={e.id} className="flex items-center justify-between gap-2 py-1.5">
            <span>
              {LABEL[e.format]} · {formatRelativeTime(e.createdAt)} ·{' '}
              {e.status === 'running' || e.status === 'queued' ? (
                `${e.status} ${e.total ? `${Math.floor((e.done / e.total) * 100)}% (${e.done}/${e.total})` : ''}`
              ) : e.status === 'succeeded' ? (
                `${e.total} rows, ${formatBytes(e.size)} compressed`
              ) : e.status === 'failed' ? (
                <span className="text-error">failed: {e.error}</span>
              ) : (
                'expired'
              )}
            </span>
            {e.status === 'succeeded' && (
              <span className="flex gap-1">
                <Button size="sm" variant="ghost" onClick={() => window.open(`/api/exports/${e.id}/download`, '_blank')}>
                  Download
                </Button>
                <Button size="sm" variant="ghost" onClick={() => window.open(`/api/exports/${e.id}/download?gz=1`, '_blank')}>
                  .gz
                </Button>
              </span>
            )}
          </li>
        ))}
      </ul>
    </div>
  );
}
