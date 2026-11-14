'use client';

/** Audit → SIEM: forwarding destinations (syslog, Splunk HEC, HTTPS, JSON lines) with backlog and errors. */
import React, { useState } from 'react';
import { Dialog, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { ErrorBanner } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { useAuthStore } from '@/lib/stores/auth-store';
import { formatRelativeTime } from '@/lib/utils/format';
import type { SiemDestination, SiemKind } from '@/lib/db/repos/siem';

type Row = SiemDestination & { backlog: number };

const KIND_LABELS: Record<SiemKind, string> = { syslog: 'Syslog (RFC 5424)', hec: 'Splunk HTTP Event Collector', https: 'HTTPS (JSON batches)', jsonl: 'JSON lines file' };

interface Draft {
  name: string;
  kind: SiemKind;
  host: string;
  port: string;
  transport: 'udp' | 'tcp' | 'tls';
  caPem: string;
  url: string;
  index: string;
  file: string;
  secret: string;
  includeChildren: boolean;
  backfill: boolean;
}
const EMPTY: Draft = { name: '', kind: 'syslog', host: '', port: '', transport: 'tls', caPem: '', url: '', index: '', file: 'ufop-audit.jsonl', secret: '', includeChildren: false, backfill: false };

function config(d: Draft): Record<string, unknown> {
  if (d.kind === 'syslog') return { host: d.host, port: d.port ? Number(d.port) : undefined, transport: d.transport, caPem: d.caPem || null };
  if (d.kind === 'hec') return { url: d.url, index: d.index || undefined };
  if (d.kind === 'https') return { url: d.url };
  return { file: d.file };
}

export function SiemDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const { data, error, reload } = useApi<{ destinations: Row[]; jsonlAvailable: boolean }>(open ? '/api/siem' : null, 15_000);
  const canWrite = useAuthStore((s) => s.checkPermission)('audit.export');
  const [draft, setDraft] = useState<Draft | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [tested, setTested] = useState<Record<string, string>>({});
  const run = async (f: () => Promise<unknown>) => {
    setErr(null);
    try {
      await f();
      await reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };
  const create = () =>
    run(async () => {
      if (!draft) return;
      await api('/api/siem', { body: { name: draft.name, kind: draft.kind, config: config(draft), secret: draft.secret || undefined, includeChildren: draft.includeChildren, backfill: draft.backfill } });
      setDraft(null);
    });
  const test = async (d: Row) => {
    const r = await fetch(`/api/siem/${d.id}/test`, { method: 'POST' }).then((x) => x.json() as Promise<{ ok: boolean; error: string | null }>);
    setTested({ ...tested, [d.id]: r.ok ? 'Test event sent' : `Failed: ${r.error}` });
  };

  return (
    <Dialog open={open} onClose={onClose} className="max-w-2xl">
      <DialogHeader>
        <DialogTitle>SIEM forwarding</DialogTitle>
        <DialogDescription>Every audit entry is forwarded in order, with its hash-chain fields. A destination that is down keeps its place and catches up when it is back.</DialogDescription>
      </DialogHeader>
      <ErrorBanner message={err ?? error} />
      <ul className="mb-3 space-y-2 text-sm">
        {data?.destinations.map((d) => (
          <li key={d.id} className="rounded-md border border-border p-2">
            <div className="flex items-center justify-between gap-2">
              <span>
                <strong>{d.name}</strong>{' '}
                <span className="text-xs text-foreground-tertiary">
                  {KIND_LABELS[d.kind]}
                  {d.includeChildren ? ' · with child organizations' : ''}
                </span>
              </span>
              {canWrite && (
                <span className="flex gap-1">
                  <Button size="sm" variant="ghost" onClick={() => void test(d)}>
                    Test
                  </Button>
                  <Button size="sm" variant="ghost" onClick={() => void run(() => api(`/api/siem/${d.id}`, { method: 'PATCH', body: { enabled: !d.enabled } }))}>
                    {d.enabled ? 'Pause' : 'Resume'}
                  </Button>
                  <Button size="sm" variant="ghost" onClick={() => void run(() => api(`/api/siem/${d.id}`, { method: 'DELETE' }))}>
                    Remove
                  </Button>
                </span>
              )}
            </div>
            <p className="text-xs text-foreground-secondary">
              {d.enabled ? '' : 'Paused · '}
              {d.backlog === 0 ? 'Up to date' : `${d.backlog} entries waiting`} · {d.forwardedTotal} forwarded
              {d.lastSuccessAt ? ` · last sent ${formatRelativeTime(d.lastSuccessAt)}` : ''}
            </p>
            {d.lastError && (
              <p className="text-xs text-error">
                {d.lastError}
                {d.nextAttemptAt ? ` · retrying ${formatRelativeTime(d.nextAttemptAt)}` : ''}
              </p>
            )}
            {tested[d.id] && <p className="text-xs">{tested[d.id]}</p>}
          </li>
        ))}
        {data && !data.destinations.length && <li className="text-foreground-tertiary">No destinations yet.</li>}
      </ul>
      {canWrite && !draft && (
        <Button size="sm" onClick={() => setDraft(EMPTY)}>
          Add destination
        </Button>
      )}
      {draft && (
        <div className="space-y-2">
          <Input label="Name" value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} />
          <Select
            label="Kind"
            options={(Object.keys(KIND_LABELS) as SiemKind[]).filter((k) => k !== 'jsonl' || data?.jsonlAvailable).map((k) => ({ value: k, label: KIND_LABELS[k] }))}
            value={draft.kind}
            onChange={(e) => setDraft({ ...draft, kind: e.target.value as SiemKind })}
          />
          {draft.kind === 'syslog' && (
            <>
              <div className="grid grid-cols-3 gap-2">
                <Input label="Server" value={draft.host} onChange={(e) => setDraft({ ...draft, host: e.target.value })} placeholder="siem.example.com" />
                <Input label="Port" value={draft.port} onChange={(e) => setDraft({ ...draft, port: e.target.value })} placeholder={draft.transport === 'tls' ? '6514' : '514'} />
                <Select
                  label="Transport"
                  options={[
                    { value: 'tls', label: 'TLS' },
                    { value: 'tcp', label: 'TCP' },
                    { value: 'udp', label: 'UDP' },
                  ]}
                  value={draft.transport}
                  onChange={(e) => setDraft({ ...draft, transport: e.target.value as Draft['transport'] })}
                />
              </div>
              {draft.transport === 'tls' && (
                <label className="flex flex-col gap-1 text-sm">
                  Trusted CA (PEM, optional)
                  <textarea className="min-h-16 rounded-md border border-border bg-background p-2 font-mono text-xs" value={draft.caPem} onChange={(e) => setDraft({ ...draft, caPem: e.target.value })} />
                </label>
              )}
            </>
          )}
          {(draft.kind === 'hec' || draft.kind === 'https') && (
            <Input
              label="URL"
              value={draft.url}
              onChange={(e) => setDraft({ ...draft, url: e.target.value })}
              placeholder={draft.kind === 'hec' ? 'https://splunk.example.com:8088/services/collector/event' : 'https://collector.example.com/ingest'}
            />
          )}
          {draft.kind === 'hec' && <Input label="Index (optional)" value={draft.index} onChange={(e) => setDraft({ ...draft, index: e.target.value })} />}
          {(draft.kind === 'hec' || draft.kind === 'https') && (
            <Input label={draft.kind === 'hec' ? 'HEC token' : 'Bearer token / signing secret (optional)'} type="password" value={draft.secret} onChange={(e) => setDraft({ ...draft, secret: e.target.value })} />
          )}
          {draft.kind === 'jsonl' && <Input label="File name (in SIEM_JSONL_DIR)" value={draft.file} onChange={(e) => setDraft({ ...draft, file: e.target.value })} />}
          <label className="flex items-center gap-2 text-sm">
            <input type="checkbox" checked={draft.includeChildren} onChange={(e) => setDraft({ ...draft, includeChildren: e.target.checked })} />
            Include child organizations
          </label>
          <label className="flex items-center gap-2 text-sm">
            <input type="checkbox" checked={draft.backfill} onChange={(e) => setDraft({ ...draft, backfill: e.target.checked })} />
            Also send the existing history (otherwise from now on)
          </label>
          <div className="flex gap-2">
            <Button size="sm" variant="secondary" onClick={() => setDraft(null)}>
              Cancel
            </Button>
            <Button size="sm" disabled={!draft.name.trim()} onClick={() => void create()}>
              Add
            </Button>
          </div>
        </div>
      )}
      <DialogFooter>
        <Button variant="outline" onClick={onClose}>
          Close
        </Button>
      </DialogFooter>
    </Dialog>
  );
}
