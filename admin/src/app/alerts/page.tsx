'use client';

import React, { useState } from 'react';
import Link from 'next/link';
import { Bell, Plus, Send } from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { Tabs, TabContent } from '@/components/ui/tabs';
import { EmptyState } from '@/components/ui/empty-state';
import { Dialog, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table';
import { ErrorBanner, Loading, StatusBadge, TagChip } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { useAuthStore } from '@/lib/stores/auth-store';
import { useToastStore } from '@/lib/stores/toast-store';
import { formatRelativeTime } from '@/lib/utils/format';
import type { Alert, AlertKind, AlertParams, AlertRule } from '@/lib/db/repos/alerts';
import type { Channel, ChannelKind, EmailConfig, WebhookConfig } from '@/lib/db/repos/channels';

const KIND_LABEL: Record<AlertKind, string> = {
  backup_failed: 'Backup failed',
  missed_backup: 'Missed backup',
  changed_files: 'Unusual number of changed files',
  duration_anomaly: 'Unusual duration',
};

const DEFAULT_PARAMS: Record<AlertKind, AlertParams> = {
  backup_failed: { includeWarnings: false, notifyOnResolve: true },
  missed_backup: { graceMinutes: 60, defaultIntervalHours: 24, notifyOnResolve: true },
  changed_files: { maxChangedPercent: 30, maxDeletedFiles: 1000 },
  duration_anomaly: { minSamples: 5, factor: 2, stdDevs: 3, detectShort: false },
};

const NUMERIC: Record<AlertKind, [keyof AlertParams, string][]> = {
  backup_failed: [],
  missed_backup: [
    ['graceMinutes', 'Grace period (minutes)'],
    ['defaultIntervalHours', 'Assumed interval when the client reports none (hours, 0 = skip)'],
  ],
  changed_files: [
    ['maxChangedFiles', 'Changed files above'],
    ['maxChangedPercent', 'Changed share above (%)'],
    ['maxDeletedFiles', 'Deleted files above'],
  ],
  duration_anomaly: [
    ['minSamples', 'Earlier runs needed'],
    ['factor', 'Times the median'],
    ['stdDevs', 'Standard deviations'],
  ],
};

interface RuleDraft {
  id: string | null;
  name: string;
  kind: AlertKind;
  params: AlertParams;
  tags: string;
  channelIds: string[];
  cooldownMinutes: number;
  enabled: boolean;
}

interface ChannelDraft {
  id: string | null;
  name: string;
  kind: ChannelKind;
  email: EmailConfig;
  url: string;
  secret: string;
  enabled: boolean;
  hasSecret: boolean;
}

const emptyEmail: EmailConfig = { host: '', port: 587, security: 'starttls', username: '', from: '', to: [], caPem: null, useForInvitations: false };

export default function AlertsPage() {
  const check = useAuthStore((s) => s.checkPermission);
  const notify = useToastStore((s) => s.notify);
  const canWrite = check('alerts.write');
  const [tab, setTab] = useState('alerts');
  const [state, setState] = useState('open');
  const alerts = useApi<{ alerts: Alert[] }>(`/api/alerts?state=${state}`, 30_000);
  const rules = useApi<{ rules: AlertRule[] }>('/api/alerts/rules');
  const channels = useApi<{ channels: Channel[] }>('/api/alerts/channels');
  const [err, setErr] = useState<string | null>(null);
  const [rule, setRule] = useState<RuleDraft | null>(null);
  const [channel, setChannel] = useState<ChannelDraft | null>(null);
  const [formErr, setFormErr] = useState<string | null>(null);

  const act = async (fn: () => Promise<unknown>, after: () => Promise<void>) => {
    setErr(null);
    try {
      await fn();
      await after();
    } catch (e) {
      setErr(errorText(e));
    }
  };

  const saveRule = async () => {
    if (!rule) return;
    setFormErr(null);
    const body = { name: rule.name, kind: rule.kind, params: rule.params, scope: { tags: rule.tags.split(',').map((t) => t.trim()).filter(Boolean), machineIds: [] }, channelIds: rule.channelIds, cooldownMinutes: rule.cooldownMinutes, enabled: rule.enabled };
    try {
      await api(rule.id ? `/api/alerts/rules/${rule.id}` : '/api/alerts/rules', { method: rule.id ? 'PUT' : 'POST', body });
      setRule(null);
      await rules.reload();
    } catch (e) {
      setFormErr(errorText(e));
    }
  };

  const saveChannel = async () => {
    if (!channel) return;
    setFormErr(null);
    const config = channel.kind === 'email' ? channel.email : channel.kind === 'webhook' ? ({ url: channel.url } as WebhookConfig) : {};
    const body: Record<string, unknown> = { name: channel.name, kind: channel.kind, config, enabled: channel.enabled };
    if (channel.secret) body.secret = channel.secret;
    try {
      await api(channel.id ? `/api/alerts/channels/${channel.id}` : '/api/alerts/channels', { method: channel.id ? 'PUT' : 'POST', body });
      setChannel(null);
      await channels.reload();
    } catch (e) {
      setFormErr(errorText(e));
    }
  };

  const test = async (c: Channel) => {
    try {
      await api(`/api/alerts/channels/${c.id}/test`, { method: 'POST', body: {} });
      notify(`Test sent through "${c.name}"`, 'success');
    } catch (e) {
      notify(errorText(e), 'error');
    }
    await channels.reload();
  };

  const secretLabel = (k: ChannelKind) => (k === 'email' ? 'SMTP password' : k === 'slack' ? 'Slack incoming webhook URL' : 'Signing secret (HMAC-SHA256)');

  return (
    <div>
      <PageHeader
        title="Alerts"
        description="Failed, missed or unusual backups, and where notifications go."
        actions={
          canWrite && tab === 'rules' ? (
            <Button onClick={() => (setFormErr(null), setRule({ id: null, name: '', kind: 'backup_failed', params: DEFAULT_PARAMS.backup_failed, tags: '', channelIds: [], cooldownMinutes: 360, enabled: true }))}>
              <Plus size={14} className="mr-2" aria-hidden="true" />
              New rule
            </Button>
          ) : canWrite && tab === 'channels' ? (
            <Button onClick={() => (setFormErr(null), setChannel({ id: null, name: '', kind: 'email', email: emptyEmail, url: '', secret: '', enabled: true, hasSecret: false }))}>
              <Plus size={14} className="mr-2" aria-hidden="true" />
              New channel
            </Button>
          ) : undefined
        }
      />
      <ErrorBanner message={err ?? alerts.error ?? rules.error ?? channels.error} />
      <Tabs
        activeTab={tab}
        onTabChange={setTab}
        tabs={[
          { id: 'alerts', label: 'Alerts', count: state === 'open' ? alerts.data?.alerts.length : undefined },
          { id: 'rules', label: 'Rules', count: rules.data?.rules.length },
          { id: 'channels', label: 'Channels', count: channels.data?.channels.length },
        ]}
      />

      <TabContent tabId="alerts" activeTab={tab}>
        <div className="mb-3 w-48">
          <Select aria-label="Show" value={state} onChange={(e) => setState(e.target.value)} options={[{ value: 'open', label: 'Open' }, { value: 'resolved', label: 'Resolved' }, { value: 'all', label: 'All' }]} />
        </div>
        {alerts.loading && !alerts.data ? (
          <Loading />
        ) : !alerts.data?.alerts.length ? (
          <Card>
            <EmptyState icon={Bell} title={state === 'open' ? 'No open alerts' : 'No alerts'} description="Alerts appear here when a rule matches a report or a backup misses its schedule." />
          </Card>
        ) : (
          <Card padding="none">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Alert</TableHead>
                  <TableHead>Severity</TableHead>
                  <TableHead className="hidden md:table-cell">Raised</TableHead>
                  <TableHead className="hidden md:table-cell">Notified</TableHead>
                  <TableHead />
                </TableRow>
              </TableHeader>
              <TableBody>
                {alerts.data.alerts.map((a) => (
                  <TableRow key={a.id}>
                    <TableCell>
                      <p className={a.resolvedAt ? 'text-foreground-secondary' : 'font-medium'}>{a.message}</p>
                      <p className="text-xs text-foreground-tertiary">
                        {KIND_LABEL[a.kind]}
                        {a.machineId && (
                          <>
                            {' · '}
                            <Link href={`/fleet/machines/${a.machineId}`} className="text-primary">
                              machine
                            </Link>
                          </>
                        )}
                        {a.resolvedAt ? ` · resolved ${formatRelativeTime(a.resolvedAt)}` : ''}
                      </p>
                    </TableCell>
                    <TableCell>
                      <StatusBadge value={a.severity === 'critical' ? 'error' : 'warning'} />
                    </TableCell>
                    <TableCell className="hidden md:table-cell">{formatRelativeTime(a.createdAt)}</TableCell>
                    <TableCell className="hidden md:table-cell text-xs">
                      {a.notified.length === 0 ? '—' : `${a.notified.filter((n) => n.ok).length}/${a.notified.length} delivered`}
                      {a.notified.some((n) => !n.ok) && <p className="text-error">{a.notified.filter((n) => !n.ok).slice(-1)[0]?.error}</p>}
                    </TableCell>
                    <TableCell className="text-right">
                      {canWrite && !a.resolvedAt && (
                        <Button size="sm" variant="ghost" onClick={() => void act(() => api(`/api/alerts/${a.id}`, { method: 'PATCH', body: {} }), alerts.reload)}>
                          Resolve
                        </Button>
                      )}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </Card>
        )}
      </TabContent>

      <TabContent tabId="rules" activeTab={tab}>
        {!rules.data?.rules.length ? (
          <Card>
            <EmptyState icon={Bell} title="No alert rules" description="Add a rule to be told when backups fail, miss their schedule or behave unusually." />
          </Card>
        ) : (
          <div className="space-y-3">
            {rules.data.rules.map((r) => (
              <Card key={r.id} padding="sm">
                <div className="flex flex-col gap-2 md:flex-row md:items-center md:justify-between">
                  <div>
                    <p className="font-medium">
                      {r.name} {!r.enabled && <span className="text-xs text-warning">disabled</span>}
                    </p>
                    <p className="text-xs text-foreground-secondary">
                      {KIND_LABEL[r.kind]} · {r.channelIds.length} channel(s) · re-notify after {r.cooldownMinutes} min
                    </p>
                    <div className="mt-1 flex flex-wrap gap-1">
                      {r.scope.tags.length ? r.scope.tags.map((t) => <TagChip key={t} tag={t} />) : <span className="text-xs text-foreground-tertiary">all machines</span>}
                    </div>
                  </div>
                  {canWrite && (
                    <div className="flex gap-2">
                      <Button size="sm" variant="outline" onClick={() => (setFormErr(null), setRule({ id: r.id, name: r.name, kind: r.kind, params: r.params, tags: r.scope.tags.join(', '), channelIds: r.channelIds, cooldownMinutes: r.cooldownMinutes, enabled: r.enabled }))}>
                        Edit
                      </Button>
                      <Button size="sm" variant="ghost" onClick={() => void act(() => api(`/api/alerts/rules/${r.id}`, { method: 'DELETE' }), rules.reload)}>
                        Delete
                      </Button>
                    </div>
                  )}
                </div>
              </Card>
            ))}
          </div>
        )}
      </TabContent>

      <TabContent tabId="channels" activeTab={tab}>
        {!channels.data?.channels.length ? (
          <Card>
            <EmptyState icon={Send} title="No channels" description="Add an email (SMTP), Slack or webhook channel for alert rules to use." />
          </Card>
        ) : (
          <div className="space-y-3">
            {channels.data.channels.map((c) => (
              <Card key={c.id} padding="sm">
                <div className="flex flex-col gap-2 md:flex-row md:items-center md:justify-between">
                  <div>
                    <p className="font-medium">
                      {c.name} <span className="text-xs text-foreground-tertiary">{c.kind}</span> {!c.enabled && <span className="text-xs text-warning">disabled</span>}
                    </p>
                    <p className="text-xs text-foreground-secondary">
                      {c.kind === 'email' ? `${(c.config as EmailConfig).host} → ${(c.config as EmailConfig).to?.join(', ')}` : c.kind === 'webhook' ? (c.config as WebhookConfig).url : 'Slack incoming webhook'}
                    </p>
                    {c.lastError ? <p className="text-xs text-error">Last error: {c.lastError}</p> : c.lastSentAt ? <p className="text-xs text-foreground-tertiary">Last sent {formatRelativeTime(c.lastSentAt)}</p> : null}
                  </div>
                  {canWrite && (
                    <div className="flex gap-2">
                      <Button size="sm" variant="outline" onClick={() => void test(c)}>
                        Send test
                      </Button>
                      <Button
                        size="sm"
                        variant="outline"
                        onClick={() =>
                          (setFormErr(null),
                          setChannel({ id: c.id, name: c.name, kind: c.kind, email: c.kind === 'email' ? { ...emptyEmail, ...(c.config as EmailConfig) } : emptyEmail, url: c.kind === 'webhook' ? (c.config as WebhookConfig).url : '', secret: '', enabled: c.enabled, hasSecret: c.hasSecret }))
                        }
                      >
                        Edit
                      </Button>
                      <Button size="sm" variant="ghost" onClick={() => void act(() => api(`/api/alerts/channels/${c.id}`, { method: 'DELETE' }), channels.reload)}>
                        Delete
                      </Button>
                    </div>
                  )}
                </div>
              </Card>
            ))}
          </div>
        )}
      </TabContent>

      <Dialog open={!!rule} onClose={() => setRule(null)} className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>{rule?.id ? 'Edit rule' : 'New alert rule'}</DialogTitle>
        </DialogHeader>
        {rule && (
          <div className="max-h-[60vh] space-y-3 overflow-y-auto pr-1">
            <ErrorBanner message={formErr} />
            <Input label="Name" value={rule.name} onChange={(e) => setRule({ ...rule, name: e.target.value })} />
            <Select
              label="Trigger"
              value={rule.kind}
              disabled={!!rule.id}
              onChange={(e) => {
                const kind = e.target.value as AlertKind;
                setRule({ ...rule, kind, params: DEFAULT_PARAMS[kind] });
              }}
              options={(Object.keys(KIND_LABEL) as AlertKind[]).map((k) => ({ value: k, label: KIND_LABEL[k] }))}
            />
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              {NUMERIC[rule.kind].map(([k, label]) => (
                <Input key={k} label={label} type="number" min={0} value={(rule.params[k] as number | undefined) ?? ''} onChange={(e) => setRule({ ...rule, params: { ...rule.params, [k]: e.target.value === '' ? undefined : Number(e.target.value) } })} />
              ))}
            </div>
            <div className="flex flex-wrap gap-4 text-sm">
              {rule.kind === 'backup_failed' && (
                <label className="flex items-center gap-2">
                  <input type="checkbox" checked={!!rule.params.includeWarnings} onChange={(e) => setRule({ ...rule, params: { ...rule.params, includeWarnings: e.target.checked } })} />
                  Also on warnings
                </label>
              )}
              {rule.kind === 'duration_anomaly' && (
                <label className="flex items-center gap-2">
                  <input type="checkbox" checked={!!rule.params.detectShort} onChange={(e) => setRule({ ...rule, params: { ...rule.params, detectShort: e.target.checked } })} />
                  Also when much shorter
                </label>
              )}
              <label className="flex items-center gap-2">
                <input type="checkbox" checked={!!rule.params.notifyOnResolve} onChange={(e) => setRule({ ...rule, params: { ...rule.params, notifyOnResolve: e.target.checked } })} />
                Notify when resolved
              </label>
              <label className="flex items-center gap-2">
                <input type="checkbox" checked={rule.enabled} onChange={(e) => setRule({ ...rule, enabled: e.target.checked })} />
                Enabled
              </label>
            </div>
            <Input label="Only machines with all of these tags" placeholder="Empty = every machine" value={rule.tags} onChange={(e) => setRule({ ...rule, tags: e.target.value })} />
            <Input label="Re-notify an open alert after (minutes)" type="number" min={0} value={rule.cooldownMinutes} onChange={(e) => setRule({ ...rule, cooldownMinutes: Number(e.target.value) })} />
            <fieldset>
              <legend className="mb-1 text-sm font-medium">Notify through</legend>
              {channels.data?.channels.map((c) => (
                <label key={c.id} className="mr-4 inline-flex items-center gap-2 text-sm">
                  <input type="checkbox" checked={rule.channelIds.includes(c.id)} onChange={(e) => setRule({ ...rule, channelIds: e.target.checked ? [...rule.channelIds, c.id] : rule.channelIds.filter((x) => x !== c.id) })} />
                  {c.name}
                </label>
              ))}
              {!channels.data?.channels.length && <p className="text-xs text-foreground-secondary">No channels yet. Alerts are still listed here.</p>}
            </fieldset>
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => setRule(null)}>
            Cancel
          </Button>
          <Button onClick={() => void saveRule()}>Save</Button>
        </DialogFooter>
      </Dialog>

      <Dialog open={!!channel} onClose={() => setChannel(null)} className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>{channel?.id ? 'Edit channel' : 'New channel'}</DialogTitle>
          <DialogDescription>Secrets are stored encrypted and never shown again; leave the field empty to keep the current one.</DialogDescription>
        </DialogHeader>
        {channel && (
          <div className="max-h-[60vh] space-y-3 overflow-y-auto pr-1">
            <ErrorBanner message={formErr} />
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <Input label="Name" value={channel.name} onChange={(e) => setChannel({ ...channel, name: e.target.value })} />
              <Select label="Type" value={channel.kind} disabled={!!channel.id} onChange={(e) => setChannel({ ...channel, kind: e.target.value as ChannelKind })} options={[{ value: 'email', label: 'Email (SMTP)' }, { value: 'slack', label: 'Slack' }, { value: 'webhook', label: 'Webhook' }]} />
            </div>
            {channel.kind === 'email' && (
              <>
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
                  <Input label="SMTP host" value={channel.email.host} onChange={(e) => setChannel({ ...channel, email: { ...channel.email, host: e.target.value } })} />
                  <Input label="Port" type="number" value={channel.email.port} onChange={(e) => setChannel({ ...channel, email: { ...channel.email, port: Number(e.target.value) } })} />
                  <Select label="Security" value={channel.email.security} onChange={(e) => setChannel({ ...channel, email: { ...channel.email, security: e.target.value as EmailConfig['security'] } })} options={[{ value: 'starttls', label: 'STARTTLS' }, { value: 'tls', label: 'TLS' }, { value: 'none', label: 'None (local relay)' }]} />
                </div>
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                  <Input label="Username" value={channel.email.username} onChange={(e) => setChannel({ ...channel, email: { ...channel.email, username: e.target.value } })} autoComplete="off" />
                  <Input label="From" value={channel.email.from} onChange={(e) => setChannel({ ...channel, email: { ...channel.email, from: e.target.value } })} />
                </div>
                <Input label="To (comma separated)" value={channel.email.to.join(', ')} onChange={(e) => setChannel({ ...channel, email: { ...channel.email, to: e.target.value.split(',').map((x) => x.trim()).filter(Boolean) } })} />
                <label className="flex flex-col gap-1.5 text-sm font-medium text-foreground">
                  Trusted server certificate (PEM, optional)
                  <textarea className="min-h-20 rounded-md border border-border bg-background p-2 font-mono text-xs" value={channel.email.caPem ?? ''} onChange={(e) => setChannel({ ...channel, email: { ...channel.email, caPem: e.target.value || null } })} spellCheck={false} />
                </label>
                <label className="flex items-center gap-2 text-sm">
                  <input type="checkbox" checked={!!channel.email.useForInvitations} onChange={(e) => setChannel({ ...channel, email: { ...channel.email, useForInvitations: e.target.checked } })} />
                  Also send invitation emails through this channel
                </label>
              </>
            )}
            {channel.kind === 'webhook' && <Input label="URL" placeholder="https://" value={channel.url} onChange={(e) => setChannel({ ...channel, url: e.target.value })} />}
            <Input label={secretLabel(channel.kind)} type="password" autoComplete="new-password" placeholder={channel.hasSecret ? '•••••• (unchanged)' : ''} value={channel.secret} onChange={(e) => setChannel({ ...channel, secret: e.target.value })} />
            <label className="flex items-center gap-2 text-sm">
              <input type="checkbox" checked={channel.enabled} onChange={(e) => setChannel({ ...channel, enabled: e.target.checked })} />
              Enabled
            </label>
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => setChannel(null)}>
            Cancel
          </Button>
          <Button onClick={() => void saveChannel()}>Save</Button>
        </DialogFooter>
      </Dialog>
    </div>
  );
}
