'use client';

/** Alerts → Deliveries: every notification sent through a channel, with retries. */
import React, { useState } from 'react';
import { Button } from '@/components/ui/button';
import { Card } from '@/components/ui/card';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table';
import { ErrorBanner, StatusBadge } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { formatRelativeTime } from '@/lib/utils/format';
import type { OutboxEntry, OutboxStatus } from '@/lib/db/repos/outbox';

type Entry = Omit<OutboxEntry, 'message'> & { message: { subject: string; recipients: number | null } };
interface Page {
  counts: Record<OutboxStatus, number>;
  channels: Record<string, { name: string; kind: string }>;
  messages: Entry[];
}

const LABEL: Record<OutboxStatus, string> = { pending: 'retrying', sent: 'sent', dead: 'failed', discarded: 'discarded' };

export function OutboxView({ canWrite }: { canWrite: boolean }) {
  const [status, setStatus] = useState<'' | OutboxStatus>('');
  const { data, error, reload } = useApi<Page>(`/api/alerts/outbox${status ? `?status=${status}` : ''}`, 15_000);
  const [err, setErr] = useState<string | null>(null);
  const act = async (id: string, action: 'retry' | 'discard') => {
    setErr(null);
    try {
      const r = await api<{ ok: boolean; error?: string | null }>(`/api/alerts/outbox/${id}`, { body: { action } });
      if (action === 'retry' && !r.ok) setErr(`Still failing: ${r.error ?? 'unknown error'}`);
      await reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };
  return (
    <div className="space-y-3">
      <ErrorBanner message={err ?? error} />
      <div className="flex flex-wrap gap-1 text-xs" role="group" aria-label="Filter deliveries">
        {(['', 'pending', 'dead', 'sent', 'discarded'] as const).map((s) => (
          <button key={s || 'all'} type="button" aria-pressed={status === s} onClick={() => setStatus(s)} className={`rounded border px-2 py-0.5 ${status === s ? 'border-primary bg-primary text-white' : 'border-border'}`}>
            {s ? `${LABEL[s]} (${data?.counts[s] ?? 0})` : 'all'}
          </button>
        ))}
      </div>
      <Card padding="none">
        {!data?.messages.length ? (
          <p className="p-6 text-sm text-foreground-secondary">No notifications{status && ` ${LABEL[status]}`}.</p>
        ) : (
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Message</TableHead>
                <TableHead>Channel</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>Last error</TableHead>
                <TableHead />
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.messages.map((m) => (
                <TableRow key={m.id}>
                  <TableCell className="text-xs">
                    <p className="font-medium">{m.message.subject}</p>
                    <p className="text-foreground-tertiary">
                      {formatRelativeTime(m.createdAt)}
                      {m.context && ` · ${m.context}`}
                    </p>
                  </TableCell>
                  <TableCell className="text-xs">{data.channels[m.channelId]?.name ?? m.channelId} <span className="text-foreground-tertiary">({m.kind})</span></TableCell>
                  <TableCell className="text-xs">
                    <StatusBadge value={m.status === 'pending' ? 'pending' : m.status === 'dead' ? 'failed' : m.status === 'sent' ? 'succeeded' : 'cancelled'} />
                    <p className="text-foreground-tertiary">
                      {m.attempts} attempt{m.attempts === 1 ? '' : 's'}
                      {m.status === 'pending' && m.nextAttemptAt && ` · next ${formatRelativeTime(m.nextAttemptAt)}`}
                    </p>
                  </TableCell>
                  <TableCell className="max-w-xs truncate text-xs" title={m.lastError ?? ''}>
                    {m.lastError ?? '—'}
                  </TableCell>
                  <TableCell className="whitespace-nowrap text-right">
                    {canWrite && m.status !== 'sent' && (
                      <>
                        <Button size="sm" variant="ghost" onClick={() => void act(m.id, 'retry')}>
                          Retry now
                        </Button>
                        {(m.status === 'pending' || m.status === 'dead') && (
                          <Button size="sm" variant="ghost" onClick={() => void act(m.id, 'discard')}>
                            Discard
                          </Button>
                        )}
                      </>
                    )}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </Card>
    </div>
  );
}
