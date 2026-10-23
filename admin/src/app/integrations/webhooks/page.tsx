'use client';

/**
 * Outbound webhooks: subscribe URLs to console events, see every delivery
 * (status, attempts, response), send tests, redeliver, pause and resume.
 */
import React, { useState } from 'react';
import { Plus, Send, Webhook as WebhookIcon } from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { EmptyState } from '@/components/ui/empty-state';
import { Dialog, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table';
import { ErrorBanner, Loading, StatusBadge } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { useAuthStore } from '@/lib/stores/auth-store';
import { formatRelativeTime } from '@/lib/utils/format';
import type { DeliveryStatus, Webhook, WebhookDelivery } from '@/lib/db/repos/webhooks';

type Row = Webhook & { stats: Record<DeliveryStatus, number> };
interface Draft {
  id: string | null;
  name: string;
  url: string;
  events: string[];
  custom: string;
  headers: string;
}

const emptyDraft: Draft = { id: null, name: '', url: 'https://', events: [], custom: '', headers: '' };

function parseHeaders(text: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of text.split('\n')) {
    const i = line.indexOf(':');
    if (i > 0) out[line.slice(0, i).trim()] = line.slice(i + 1).trim();
  }
  return out;
}

function Deliveries({ hook, canWrite, onError }: { hook: Row; canWrite: boolean; onError: (e: string) => void }) {
  const [status, setStatus] = useState<'' | DeliveryStatus>('');
  const { data, reload } = useApi<{ deliveries: WebhookDelivery[] }>(`/api/webhooks/${hook.id}${status ? `?status=${status}` : ''}`, 10_000);
  const [open, setOpen] = useState<string | null>(null);
  const act = async (body: Record<string, unknown>) => {
    try {
      await api(`/api/webhooks/${hook.id}`, { body });
      await reload();
    } catch (e) {
      onError(errorText(e));
    }
  };
  return (
    <div className="space-y-2">
      <div className="flex flex-wrap gap-1 text-xs" role="group" aria-label="Filter deliveries">
        {(['', 'pending', 'succeeded', 'dead'] as const).map((s) => (
          <button key={s || 'all'} type="button" aria-pressed={status === s} onClick={() => setStatus(s)} className={`rounded border px-2 py-0.5 ${status === s ? 'border-primary bg-primary text-white' : 'border-border'}`}>
            {s || 'all'} {s && `(${hook.stats[s]})`}
          </button>
        ))}
      </div>
      {!data?.deliveries.length ? (
        <p className="text-xs text-foreground-tertiary">No deliveries.</p>
      ) : (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Event</TableHead>
              <TableHead>Status</TableHead>
              <TableHead>Attempts</TableHead>
              <TableHead>Response</TableHead>
              <TableHead />
            </TableRow>
          </TableHeader>
          <TableBody>
            {data.deliveries.map((d) => (
              <React.Fragment key={d.id}>
                <TableRow onClick={() => setOpen(open === d.id ? null : d.id)} className="cursor-pointer">
                  <TableCell className="text-xs">
                    <span className="font-mono">{d.event}</span>
                    <p className="text-foreground-tertiary">{formatRelativeTime(d.createdAt)}</p>
                  </TableCell>
                  <TableCell>
                    <StatusBadge value={d.status} />
                    {d.status === 'pending' && d.attempts > 0 && d.nextAttemptAt && <p className="text-xs text-foreground-tertiary">retry {formatRelativeTime(d.nextAttemptAt)}</p>}
                  </TableCell>
                  <TableCell className="text-xs">{d.attempts}</TableCell>
                  <TableCell className="max-w-xs truncate text-xs" title={d.lastError ?? ''}>
                    {d.lastStatus ?? '—'} {d.lastError && `· ${d.lastError}`} {d.durationMs !== null && `· ${d.durationMs} ms`}
                  </TableCell>
                  <TableCell className="text-right">
                    {canWrite && d.status !== 'pending' && (
                      <Button size="sm" variant="ghost" onClick={(e) => (e.stopPropagation(), void act({ action: 'redeliver', deliveryId: d.id }))}>
                        Redeliver
                      </Button>
                    )}
                  </TableCell>
                </TableRow>
                {open === d.id && (
                  <TableRow>
                    <TableCell colSpan={5}>
                      <pre className="max-h-64 overflow-auto rounded bg-background-secondary p-2 text-xs">{JSON.stringify(d.payload, null, 2)}</pre>
                    </TableCell>
                  </TableRow>
                )}
              </React.Fragment>
            ))}
          </TableBody>
        </Table>
      )}
    </div>
  );
}

export default function WebhooksPage() {
  const { data, error, loading, reload } = useApi<{ webhooks: Row[]; events: { group: string; events: string[] }[] }>('/api/webhooks', 15_000);
  const canWrite = useAuthStore((s) => s.checkPermission('alerts.write'));
  const [err, setErr] = useState<string | null>(null);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [secret, setSecret] = useState<string | null>(null);
  const [open, setOpen] = useState<string | null>(null);

  const run = async (fn: () => Promise<unknown>) => {
    setErr(null);
    try {
      await fn();
      await reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };

  const save = () =>
    run(async () => {
      if (!draft) return;
      const events = [...draft.events, ...draft.custom.split(/[\s,]+/).filter(Boolean)];
      const body = { name: draft.name, url: draft.url.trim(), events, headers: parseHeaders(draft.headers) };
      if (draft.id) await api(`/api/webhooks/${draft.id}`, { method: 'PATCH', body });
      else setSecret((await api<{ secret: string }>('/api/webhooks', { body })).secret);
      setDraft(null);
    });
  const act = (h: Row, body: Record<string, unknown>) =>
    run(async () => {
      const r = await api<{ secret?: string }>(`/api/webhooks/${h.id}`, { body });
      if (r.secret) setSecret(r.secret);
    });

  const catalog = new Set(data?.events.flatMap((g) => g.events) ?? []);
  return (
    <div>
      <PageHeader
        title="Webhooks"
        description="Send console events to other systems. Deliveries are signed, retried with backoff and logged; a webhook that keeps failing is paused."
        actions={
          canWrite && (
            <Button onClick={() => setDraft(emptyDraft)}>
              <Plus size={14} className="mr-2" aria-hidden="true" />
              Add webhook
            </Button>
          )
        }
      />
      <ErrorBanner message={err ?? error} />
      {secret && (
        <Card className="mb-4">
          <CardTitle>Signing secret</CardTitle>
          <CardDescription>Copy it now; it is not shown again. Verify X-UFOP-Signature = sha256=HMAC-SHA256(secret, &quot;&lt;X-UFOP-Timestamp&gt;.&lt;body&gt;&quot;).</CardDescription>
          <code className="mt-2 block break-all rounded bg-background-secondary p-2 text-xs">{secret}</code>
          <Button size="sm" variant="outline" className="mt-2" onClick={() => setSecret(null)}>
            Done
          </Button>
        </Card>
      )}
      {loading && !data ? (
        <Loading />
      ) : !data?.webhooks.length ? (
        <Card>
          <EmptyState icon={WebhookIcon} title="No webhooks" description="Add one to receive backup results, alerts and audit events." />
        </Card>
      ) : (
        <div className="space-y-3">
          {data.webhooks.map((h) => (
            <Card key={h.id}>
              <CardHeader>
                <div>
                  <CardTitle>
                    {h.name} {!h.enabled ? <StatusBadge value="disabled" /> : h.pausedReason ? <StatusBadge value="paused" /> : <StatusBadge value="active" />}
                  </CardTitle>
                  <p className="font-mono text-xs text-foreground-secondary">{h.url}</p>
                  <p className="text-xs text-foreground-tertiary">
                    {h.events.join(', ')} · {h.stats.succeeded} delivered, {h.stats.pending} pending, {h.stats.dead} failed
                    {h.lastFailureAt && ` · last failure ${formatRelativeTime(h.lastFailureAt)}`}
                  </p>
                  {h.pausedReason && <p className="text-xs text-amber-600">{h.pausedReason}</p>}
                </div>
                <div className="flex flex-wrap justify-end gap-1">
                  {canWrite && (
                    <>
                      <Button size="sm" variant="outline" disabled={!h.enabled || !!h.pausedReason} onClick={() => void act(h, { action: 'test' })}>
                        <Send size={12} className="mr-1" aria-hidden="true" />
                        Test
                      </Button>
                      {h.pausedReason ? (
                        <Button size="sm" variant="outline" onClick={() => void act(h, { action: 'resume' })}>
                          Resume
                        </Button>
                      ) : (
                        <Button size="sm" variant="ghost" onClick={() => void act(h, { action: 'pause' })}>
                          Pause
                        </Button>
                      )}
                      <Button
                        size="sm"
                        variant="ghost"
                        onClick={() => setDraft({ id: h.id, name: h.name, url: h.url, events: h.events.filter((e) => catalog.has(e)), custom: h.events.filter((e) => !catalog.has(e)).join(' '), headers: Object.entries(h.headers).map(([k, v]) => `${k}: ${v}`).join('\n') })}
                      >
                        Edit
                      </Button>
                      <Button size="sm" variant="ghost" onClick={() => void run(() => api(`/api/webhooks/${h.id}`, { method: 'PATCH', body: { enabled: !h.enabled } }))}>
                        {h.enabled ? 'Disable' : 'Enable'}
                      </Button>
                      <Button size="sm" variant="ghost" onClick={() => confirm('Replace the signing secret? Receivers must be updated.') && void act(h, { action: 'rotate_secret' })}>
                        New secret
                      </Button>
                      <Button size="sm" variant="ghost" onClick={() => confirm(`Delete "${h.name}" and its delivery log?`) && void run(() => api(`/api/webhooks/${h.id}`, { method: 'DELETE' }))}>
                        Delete
                      </Button>
                    </>
                  )}
                  <Button size="sm" variant="ghost" aria-expanded={open === h.id} onClick={() => setOpen(open === h.id ? null : h.id)}>
                    Deliveries
                  </Button>
                </div>
              </CardHeader>
              {open === h.id && <Deliveries hook={h} canWrite={canWrite} onError={setErr} />}
            </Card>
          ))}
        </div>
      )}

      <Dialog open={!!draft} onClose={() => setDraft(null)}>
        <DialogHeader>
          <DialogTitle>{draft?.id ? 'Edit webhook' : 'Add webhook'}</DialogTitle>
          <DialogDescription>Events are sent as JSON POST requests. Internal addresses are refused unless the console allows them.</DialogDescription>
        </DialogHeader>
        {draft && (
          <div className="max-h-[60vh] space-y-3 overflow-auto">
            <Input label="Name" value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} />
            <Input label="URL" value={draft.url} onChange={(e) => setDraft({ ...draft, url: e.target.value })} />
            <fieldset className="space-y-2">
              <legend className="text-sm font-medium">Events</legend>
              {data?.events.map((g) => (
                <div key={g.group}>
                  <p className="text-xs font-medium text-foreground-secondary">{g.group}</p>
                  <div className="flex flex-wrap gap-x-3 gap-y-1">
                    {g.events.map((e) => (
                      <label key={e} className="flex items-center gap-1 font-mono text-xs">
                        <input type="checkbox" checked={draft.events.includes(e)} onChange={(x) => setDraft({ ...draft, events: x.target.checked ? [...draft.events, e] : draft.events.filter((y) => y !== e) })} />
                        {e}
                      </label>
                    ))}
                  </div>
                </div>
              ))}
              <Input label="More events or patterns (e.g. backup.* or *)" value={draft.custom} onChange={(e) => setDraft({ ...draft, custom: e.target.value })} />
            </fieldset>
            <label className="flex flex-col gap-1.5 text-sm font-medium">
              Headers (one &quot;Name: value&quot; per line)
              <textarea className="min-h-16 rounded-md border border-border bg-background p-2 font-mono text-xs" value={draft.headers} onChange={(e) => setDraft({ ...draft, headers: e.target.value })} placeholder="Authorization: Bearer …" />
            </label>
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => setDraft(null)}>
            Cancel
          </Button>
          <Button onClick={() => void save()}>Save</Button>
        </DialogFooter>
      </Dialog>
    </div>
  );
}
