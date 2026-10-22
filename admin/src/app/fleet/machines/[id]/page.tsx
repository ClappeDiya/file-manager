'use client';

import React, { use, useState } from 'react';
import { useRouter } from 'next/navigation';
import { Play, RefreshCw, Terminal, Trash2 } from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { Tabs, TabContent } from '@/components/ui/tabs';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table';
import { ConnectivityBadge, ErrorBanner, Loading, StatusBadge, TagChip } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { useAuthStore } from '@/lib/stores/auth-store';
import { RPC_COMMANDS } from '@/lib/remote-commands';
import { formatBytes, formatDateTime, formatRelativeTime } from '@/lib/utils/format';
import type { Machine } from '@/lib/db/repos/machines';
import type { BackupRollup, StoredReport } from '@/lib/db/repos/reports';
import type { Alert } from '@/lib/db/repos/alerts';
import type { AgentCommand } from '@/lib/db/repos/commands';

interface Detail {
  machine: Machine;
  backups: BackupRollup[];
  reports: StoredReport[];
  alerts: Alert[];
  commands: AgentCommand[];
  policies: { id: string; name: string; version: number; applied: number | null }[];
}

const secs = (ms: number) => (ms >= 60_000 ? `${Math.round(ms / 60_000)} min` : `${Math.round(ms / 1000)} s`);

export default function MachineDetailPage({ params }: { params: Promise<{ id: string }> }) {
  const { id } = use(params);
  const router = useRouter();
  const { data, error, loading, reload } = useApi<Detail>(`/api/fleet/machines/${id}`, 10_000);
  const tagInfo = useApi<{ tags: { tag: string; color: string | null }[] }>('/api/fleet/tags');
  const check = useAuthStore((s) => s.checkPermission);
  const [tab, setTab] = useState('backups');
  const [err, setErr] = useState<string | null>(null);
  const [name, setName] = useState('');
  const [notes, setNotes] = useState('');
  const [newTag, setNewTag] = useState('');
  const [rpc, setRpc] = useState<string>('backup_list_jobs');
  const [args, setArgs] = useState('{}');
  const [focus, setFocus] = useState<string | null>(null);

  // Seed the editable fields once per machine, not on every poll.
  const loadedId = data?.machine.id;
  const [seeded, setSeeded] = useState<string | null>(null);
  if (data && loadedId && seeded !== loadedId) {
    setSeeded(loadedId);
    setName(data.machine.name);
    setNotes(data.machine.notes);
  }

  const run = async (fn: () => Promise<unknown>) => {
    setErr(null);
    try {
      await fn();
      await reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };
  const patch = (body: Record<string, unknown>) => run(() => api(`/api/fleet/machines/${id}`, { method: 'PATCH', body }));
  const send = (kind: string, payload: Record<string, unknown>) =>
    run(async () => {
      const r = await api<{ command: AgentCommand }>(`/api/fleet/machines/${id}/commands`, { body: { kind, payload } });
      setFocus(r.command.id);
      setTab('remote');
    });

  if (loading && !data) return <Loading />;
  if (!data) return <ErrorBanner message={error ?? 'Machine not found'} />;
  const m = data.machine;
  const colorOf = (t: string) => tagInfo.data?.tags.find((x) => x.tag === t)?.color ?? null;
  const canWrite = check('fleet.write');
  const canRemote = check('remote.execute') && m.status === 'active';
  const focused = data.commands.find((c) => c.id === focus) ?? null;

  const sendRpc = () => {
    let parsed: Record<string, unknown>;
    try {
      parsed = JSON.parse(args || '{}');
      if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) throw new Error();
    } catch {
      setErr('Arguments must be a JSON object');
      return;
    }
    void send('rpc', { command: rpc, args: parsed });
  };

  return (
    <div>
      <PageHeader
        title={m.name}
        description={`${m.hostname || 'unknown host'} · ${m.os} ${m.osVersion} ${m.arch} · ${m.agentKind} ${m.clientVersion}`}
        breadcrumbs={[{ label: 'Machines', href: '/fleet/machines' }, { label: m.name }]}
        actions={
          canWrite && (
            <div className="flex flex-wrap gap-2">
              {m.status === 'active' && (
                <Button variant="outline" onClick={() => void patch({ action: 'suspend' })}>
                  Suspend
                </Button>
              )}
              {m.status === 'suspended' && (
                <Button variant="outline" onClick={() => void patch({ action: 'resume' })}>
                  Resume
                </Button>
              )}
              {m.status === 'active' && (
                <Button variant="outline" onClick={() => void patch({ action: 'rotate_token' })}>
                  <RefreshCw size={14} className="mr-2" aria-hidden="true" />
                  Rotate token
                </Button>
              )}
              {m.status !== 'revoked' && (
                <Button
                  variant="danger"
                  onClick={() => {
                    if (confirm(`Revoke "${m.name}"? It can no longer report or receive commands.`)) void patch({ action: 'revoke' });
                  }}
                >
                  Revoke
                </Button>
              )}
              {m.status === 'revoked' && (
                <Button
                  variant="danger"
                  onClick={() =>
                    void run(async () => {
                      await api(`/api/fleet/machines/${id}`, { method: 'DELETE' });
                      router.push('/fleet/machines');
                    })
                  }
                >
                  <Trash2 size={14} className="mr-2" aria-hidden="true" />
                  Delete
                </Button>
              )}
            </div>
          )
        }
      />
      <ErrorBanner message={err ?? error} />
      <div className="mb-4 grid grid-cols-1 gap-4 lg:grid-cols-3">
        <Card>
          <CardTitle className="mb-3">Status</CardTitle>
          <dl className="space-y-2 text-sm">
            <div className="flex justify-between gap-4">
              <dt className="text-foreground-secondary">Connectivity</dt>
              <dd>
                <ConnectivityBadge value={m.connectivity} />
              </dd>
            </div>
            <div className="flex justify-between gap-4">
              <dt className="text-foreground-secondary">State</dt>
              <dd>
                <StatusBadge value={m.status} />
              </dd>
            </div>
            <div className="flex justify-between gap-4">
              <dt className="text-foreground-secondary">Last seen</dt>
              <dd className="text-right">{m.lastSeenAt ? `${formatRelativeTime(m.lastSeenAt)} from ${m.lastIp}` : 'never'}</dd>
            </div>
            <div className="flex justify-between gap-4">
              <dt className="text-foreground-secondary">Registered</dt>
              <dd className="text-right">
                {formatDateTime(m.registeredAt)} ({m.registeredVia})
              </dd>
            </div>
            <div className="flex justify-between gap-4">
              <dt className="text-foreground-secondary">Heartbeat</dt>
              <dd>every {m.heartbeatSeconds}s</dd>
            </div>
          </dl>
        </Card>
        <Card className="lg:col-span-2">
          <CardTitle className="mb-3">Details</CardTitle>
          <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
            <Input label="Display name" value={name} onChange={(e) => setName(e.target.value)} disabled={!canWrite} />
            <Input label="Notes" value={notes} onChange={(e) => setNotes(e.target.value)} disabled={!canWrite} />
          </div>
          <div className="mt-3 flex flex-wrap items-center gap-1.5">
            {m.tags.map((t) => (
              <TagChip key={t} tag={t} color={colorOf(t)} onRemove={canWrite ? () => void patch({ tags: m.tags.filter((x) => x !== t) }) : undefined} />
            ))}
            {canWrite && (
              <form
                className="flex items-center gap-1"
                onSubmit={(e) => {
                  e.preventDefault();
                  if (newTag.trim()) void patch({ tags: [...m.tags, newTag.trim()] }).then(() => setNewTag(''));
                }}
              >
                <input aria-label="New tag" className="h-7 w-28 rounded-md border border-border bg-background px-2 text-xs" placeholder="Add tag" value={newTag} onChange={(e) => setNewTag(e.target.value)} />
              </form>
            )}
          </div>
          {canWrite && (name !== m.name || notes !== m.notes) && (
            <div className="mt-3 flex justify-end">
              <Button size="sm" onClick={() => void patch({ name, notes })}>
                Save
              </Button>
            </div>
          )}
        </Card>
      </div>

      <Tabs
        activeTab={tab}
        onTabChange={setTab}
        tabs={[
          { id: 'backups', label: 'Backups', count: data.backups.length },
          { id: 'reports', label: 'Reports', count: data.reports.length },
          { id: 'alerts', label: 'Alerts', count: data.alerts.filter((a) => !a.resolvedAt).length },
          { id: 'policies', label: 'Policies', count: data.policies.length },
          ...(check('remote.execute') ? [{ id: 'remote', label: 'Remote session' }] : []),
        ]}
      />
      <TabContent tabId="backups" activeTab={tab}>
        <Card padding="none">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Backup</TableHead>
                <TableHead>Last result</TableHead>
                <TableHead>Last success</TableHead>
                <TableHead className="hidden md:table-cell">Source</TableHead>
                <TableHead className="hidden md:table-cell">Stored</TableHead>
                <TableHead className="hidden md:table-cell">Versions</TableHead>
                <TableHead />
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.backups.map((b) => {
                const job = m.jobs.find((j) => j.id === b.backupId);
                return (
                  <TableRow key={b.backupId}>
                    <TableCell className="font-medium">
                      {b.name}
                      {job?.managedByPolicy && <p className="text-xs text-foreground-tertiary">managed by policy</p>}
                    </TableCell>
                    <TableCell>
                      <StatusBadge value={b.lastStatus} />
                    </TableCell>
                    <TableCell>{b.lastSuccessAt ? formatRelativeTime(b.lastSuccessAt) : '—'}</TableCell>
                    <TableCell className="hidden md:table-cell">{formatBytes(b.sourceSize)}</TableCell>
                    <TableCell className="hidden md:table-cell">{formatBytes(b.backendSize)}</TableCell>
                    <TableCell className="hidden md:table-cell">{b.versionCount}</TableCell>
                    <TableCell>
                      {canRemote && (
                        <Button size="sm" variant="outline" onClick={() => void send('run_backup', { jobId: b.backupId })}>
                          <Play size={12} className="mr-1" aria-hidden="true" />
                          Run now
                        </Button>
                      )}
                    </TableCell>
                  </TableRow>
                );
              })}
              {m.jobs
                .filter((j) => !data.backups.some((b) => b.backupId === j.id))
                .map((j) => (
                  <TableRow key={j.id}>
                    <TableCell className="font-medium">{j.name}</TableCell>
                    <TableCell>
                      <StatusBadge value={j.lastResult} />
                    </TableCell>
                    <TableCell colSpan={4} className="text-foreground-secondary">
                      No report received yet
                    </TableCell>
                    <TableCell>
                      {canRemote && (
                        <Button size="sm" variant="outline" onClick={() => void send('run_backup', { jobId: j.id })}>
                          Run now
                        </Button>
                      )}
                    </TableCell>
                  </TableRow>
                ))}
            </TableBody>
          </Table>
        </Card>
      </TabContent>
      <TabContent tabId="reports" activeTab={tab}>
        <Card padding="none">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Finished</TableHead>
                <TableHead>Backup</TableHead>
                <TableHead>Result</TableHead>
                <TableHead>Duration</TableHead>
                <TableHead className="hidden md:table-cell">Changes</TableHead>
                <TableHead className="hidden md:table-cell">Uploaded</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.reports.map((r) => (
                <TableRow key={r.id}>
                  <TableCell>{formatDateTime(r.finishedAt)}</TableCell>
                  <TableCell>
                    {r.backupName}
                    {r.operation !== 'backup' && <span className="ml-1 text-xs text-foreground-tertiary">({r.operation})</span>}
                  </TableCell>
                  <TableCell>
                    <StatusBadge value={r.status} />
                    {r.message && <p className="max-w-xs truncate text-xs text-foreground-tertiary" title={r.message}>{r.message}</p>}
                  </TableCell>
                  <TableCell>{secs(r.durationMs)}</TableCell>
                  <TableCell className="hidden md:table-cell">
                    +{r.filesAdded} ~{r.filesModified} −{r.filesDeleted}
                  </TableCell>
                  <TableCell className="hidden md:table-cell">{formatBytes(r.bytesUploaded)}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </Card>
      </TabContent>
      <TabContent tabId="alerts" activeTab={tab}>
        <Card>
          {data.alerts.length === 0 ? (
            <p className="text-sm text-foreground-secondary">No alerts for this machine.</p>
          ) : (
            <ul className="divide-y divide-border">
              {data.alerts.map((a) => (
                <li key={a.id} className="py-2 text-sm">
                  <span className={a.resolvedAt ? 'text-foreground-secondary line-through' : 'text-foreground'}>{a.message}</span>
                  <span className="ml-2 text-xs text-foreground-tertiary">
                    {formatRelativeTime(a.createdAt)}
                    {a.resolvedAt ? ` · resolved ${formatRelativeTime(a.resolvedAt)}` : ''}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </Card>
      </TabContent>
      <TabContent tabId="policies" activeTab={tab}>
        <Card>
          {data.policies.length === 0 ? (
            <p className="text-sm text-foreground-secondary">No fleet policy targets this machine.</p>
          ) : (
            <ul className="divide-y divide-border">
              {data.policies.map((p) => (
                <li key={p.id} className="flex items-center justify-between py-2 text-sm">
                  <span>
                    {p.name} <span className="text-xs text-foreground-tertiary">v{p.version}</span>
                  </span>
                  {p.applied === p.version ? <StatusBadge value="succeeded" /> : <span className="text-xs text-warning">{p.applied ? `applied v${p.applied}, update pending` : 'not applied yet'}</span>}
                </li>
              ))}
            </ul>
          )}
          {canRemote && data.policies.length > 0 && (
            <div className="mt-3">
              <Button size="sm" variant="outline" onClick={() => void send('apply_policies', {})}>
                Apply policies now
              </Button>
            </div>
          )}
        </Card>
      </TabContent>
      <TabContent tabId="remote" activeTab={tab}>
        <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
          <Card>
            <CardHeader>
              <CardTitle>
                <Terminal size={16} className="mr-2 inline" aria-hidden="true" />
                Send a client command
              </CardTitle>
            </CardHeader>
            {!canRemote ? (
              <p className="text-sm text-foreground-secondary">Remote commands need an active machine.</p>
            ) : (
              <div className="space-y-3">
                <Select label="Command" value={rpc} onChange={(e) => setRpc(e.target.value)} options={RPC_COMMANDS.map((c) => ({ value: c, label: c }))} />
                <label className="flex flex-col gap-1.5 text-sm font-medium text-foreground">
                  Arguments (JSON)
                  <textarea className="min-h-24 rounded-md border border-border bg-background p-2 font-mono text-xs" value={args} onChange={(e) => setArgs(e.target.value)} spellCheck={false} />
                </label>
                <p className="text-xs text-foreground-tertiary">The machine runs the command on its next heartbeat (every {m.heartbeatSeconds}s) and sends back the result.</p>
                <Button onClick={sendRpc}>Send</Button>
              </div>
            )}
            {focused && (
              <div className="mt-4 rounded-md border border-border p-3">
                <p className="text-sm">
                  {focused.kind === 'rpc' ? String(focused.payload.command) : focused.kind}: <StatusBadge value={focused.status} />
                </p>
                {focused.result !== null && focused.result !== undefined && <pre className="mt-2 max-h-72 overflow-auto rounded bg-background-secondary p-2 text-xs">{JSON.stringify(focused.result, null, 2)}</pre>}
              </div>
            )}
          </Card>
          <Card padding="none">
            <div className="p-6 pb-2">
              <CardTitle>Command history</CardTitle>
            </div>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Sent</TableHead>
                  <TableHead>Command</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead />
                </TableRow>
              </TableHeader>
              <TableBody>
                {data.commands.map((c) => (
                  <TableRow key={c.id} className={c.id === focus ? 'bg-background-secondary' : ''} onClick={() => setFocus(c.id)}>
                    <TableCell className="text-xs">
                      {formatRelativeTime(c.createdAt)}
                      <p className="text-foreground-tertiary">{c.createdBy}</p>
                    </TableCell>
                    <TableCell className="font-mono text-xs">{c.kind === 'rpc' ? String(c.payload.command) : c.kind}</TableCell>
                    <TableCell>
                      <StatusBadge value={c.status} />
                    </TableCell>
                    <TableCell>
                      {c.status === 'queued' && (
                        <Button size="sm" variant="ghost" onClick={(e) => (e.stopPropagation(), void run(() => api(`/api/fleet/machines/${id}/commands?commandId=${c.id}`, { method: 'DELETE' })))}>
                          Cancel
                        </Button>
                      )}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </Card>
        </div>
      </TabContent>
    </div>
  );
}
