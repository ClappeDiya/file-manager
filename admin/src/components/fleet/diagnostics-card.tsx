'use client';

import React, { useState } from 'react';
import { Download, LifeBuoy, Trash2 } from 'lucide-react';
import { Card, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table';
import { ErrorBanner } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { formatBytes, formatDateTime } from '@/lib/utils/format';
import type { Diagnostics } from '@/lib/db/repos/diagnostics';

/**
 * Diagnostics bundles a machine uploaded (scrubbed logs, versions, OS):
 * request a fresh one through the command queue, download or delete.
 */
export function DiagnosticsCard({ machineId, canRequest, canDelete, heartbeatSeconds }: { machineId: string; canRequest: boolean; canDelete: boolean; heartbeatSeconds: number }) {
  const base = `/api/fleet/machines/${machineId}/diagnostics`;
  const { data, error, reload } = useApi<{ diagnostics: Diagnostics[] }>(base, 15_000);
  const [err, setErr] = useState<string | null>(null);
  const [requested, setRequested] = useState(false);

  const run = async (fn: () => Promise<unknown>) => {
    setErr(null);
    try {
      await fn();
      await reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };
  const request = () =>
    run(async () => {
      await api(`/api/fleet/machines/${machineId}/commands`, { body: { kind: 'collect_diagnostics', payload: {} } });
      setRequested(true);
    });

  const rows = data?.diagnostics ?? [];
  return (
    <Card padding="none">
      <div className="p-6 pb-2">
        <CardHeader>
          <CardTitle>
            <LifeBuoy size={16} className="mr-2 inline" aria-hidden="true" />
            Diagnostics
          </CardTitle>
          {canRequest && (
            <Button size="sm" variant="outline" onClick={() => void request()}>
              Request diagnostics
            </Button>
          )}
        </CardHeader>
        <p className="text-xs text-foreground-tertiary">
          Recent logs, versions and OS details. Passwords, tokens, e-mail addresses and folder names are removed on the machine before upload.
          {requested && ` The machine uploads a bundle on its next heartbeat (every ${heartbeatSeconds}s).`}
        </p>
        {(err || error) && <ErrorBanner message={err ?? error ?? ''} />}
      </div>
      {rows.length === 0 ? (
        <p className="px-6 pb-6 text-sm text-foreground-secondary">No diagnostics uploaded yet.</p>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Uploaded</TableHead>
              <TableHead>Client</TableHead>
              <TableHead>Size</TableHead>
              <TableHead />
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((d) => (
              <TableRow key={d.id}>
                <TableCell className="text-xs">
                  {formatDateTime(d.createdAt)}
                  <p className="font-mono text-foreground-tertiary">{d.fileName}</p>
                </TableCell>
                <TableCell className="text-xs">
                  {String(d.summary.appVersion ?? '—')} · {String(d.summary.os ?? '')} {String(d.summary.osVersion ?? '')}
                  {d.summary.truncated === true && <p className="text-foreground-tertiary">Older logs left out</p>}
                </TableCell>
                <TableCell className="text-xs">{formatBytes(d.size)}</TableCell>
                <TableCell className="whitespace-nowrap text-right">
                  <a href={`${base}?download=${encodeURIComponent(d.id)}`} download={d.fileName} className="inline-flex items-center gap-1 px-2 text-sm text-primary hover:underline">
                    <Download size={14} aria-hidden="true" /> Download
                  </a>
                  {canDelete && (
                    <Button size="sm" variant="ghost" aria-label={`Delete ${d.fileName}`} onClick={() => confirm(`Delete ${d.fileName}?`) && void run(() => api(`${base}?id=${encodeURIComponent(d.id)}`, { method: 'DELETE' }))}>
                      <Trash2 size={14} />
                    </Button>
                  )}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </Card>
  );
}
