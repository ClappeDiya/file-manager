'use client';

/**
 * Managed app settings for the organization's desktops: update channel,
 * automatic checks and a blackout window, the proxy, and AI. Anything left
 * on "User's choice" stays with the person using the app.
 */
import React, { useState } from 'react';
import { PageHeader } from '@/components/layout/page-header';
import { Card, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { ErrorBanner, Loading } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { useAuthStore } from '@/lib/stores/auth-store';
import { useToastStore } from '@/lib/stores/toast-store';
import { formatDateTime } from '@/lib/utils/format';
import type { ManagedSettings, ManagedSettingsBody } from '@/lib/db/repos/managed-settings';
import { MOUNT_CACHE_MODES, MOUNT_PROTOCOLS } from '@/lib/mount-profile';

const DAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
type Tri = '' | 'on' | 'off';
const tri = (v: boolean | null | undefined): Tri => (v === true ? 'on' : v === false ? 'off' : '');
const fromTri = (v: Tri) => (v === 'on' ? true : v === 'off' ? false : null);
const num = (v: string) => (v.trim() === '' ? null : Number(v));

interface Form {
  channel: '' | 'stable' | 'beta';
  autoCheck: Tri;
  blackout: boolean;
  start: string;
  end: string;
  days: number[];
  proxyMode: '' | 'system' | 'none' | 'manual' | 'pac';
  proxyUrl: string;
  pacUrl: string;
  noProxy: string;
  aiEnabled: '' | 'off';
  aiCloud: '' | 'off';
  upKbps: string;
  downKbps: string;
  runs: string;
  mountProtocols: string[] | null;
  mountReadOnly: '' | 'on';
  mountCache: '' | (typeof MOUNT_CACHE_MODES)[number];
  mountCacheMb: string;
  mountCacheHours: string;
  mountRoot: string;
  workflows: WorkflowForm[];
  backupTemplates: BackupTemplateForm[];
}

interface BackupTemplateForm {
  id: string;
  name: string;
  description: string;
  enforced: boolean;
  settingsJson: string;
}

interface WorkflowForm {
  id: string;
  name: string;
  enabled: boolean;
  ruleJson: string;
}

/** The rule as typed; text that isn't JSON goes to the server as-is so its error names the workflow. */
function parseRule(text: string): Record<string, unknown> {
  try {
    return JSON.parse(text) as Record<string, unknown>;
  } catch {
    return text as unknown as Record<string, unknown>;
  }
}

function toForm(s: ManagedSettings | null): Form {
  return {
    channel: s?.updates?.channel ?? '',
    autoCheck: tri(s?.updates?.autoCheck),
    blackout: !!s?.updates?.blackout,
    start: s?.updates?.blackout?.start ?? '08:00',
    end: s?.updates?.blackout?.end ?? '18:00',
    days: s?.updates?.blackout?.days ?? [1, 2, 3, 4, 5],
    proxyMode: s?.proxy?.mode ?? '',
    proxyUrl: s?.proxy?.url ?? '',
    pacUrl: s?.proxy?.pacUrl ?? '',
    noProxy: (s?.proxy?.noProxy ?? []).join('\n'),
    aiEnabled: s?.ai?.enabled === false ? 'off' : '',
    aiCloud: s?.ai?.allowCloud === false ? 'off' : '',
    upKbps: s?.transfers?.maxUploadKbps?.toString() ?? '',
    downKbps: s?.transfers?.maxDownloadKbps?.toString() ?? '',
    runs: s?.transfers?.maxConcurrentRuns?.toString() ?? '',
    mountProtocols: s?.mounts?.allowedProtocols ?? null,
    mountReadOnly: s?.mounts?.readOnly ? 'on' : '',
    mountCache: s?.mounts?.cacheMode ?? '',
    mountCacheMb: s?.mounts?.maxCacheMb?.toString() ?? '',
    mountCacheHours: s?.mounts?.maxCacheAgeHours?.toString() ?? '',
    mountRoot: s?.mounts?.mountRoot ?? '',
    workflows: (s?.workflows ?? []).map((w) => ({ id: w.id, name: w.name, enabled: w.enabled, ruleJson: JSON.stringify(w.rule, null, 2) })),
    backupTemplates: (s?.backupTemplates ?? []).map((t) => ({ id: t.id, name: t.name, description: t.description, enforced: t.enforced, settingsJson: JSON.stringify(t.settings, null, 2) })),
  };
}

function toBody(f: Form): ManagedSettingsBody {
  return {
    updates: { channel: f.channel || null, autoCheck: fromTri(f.autoCheck), blackout: f.blackout ? { start: f.start, end: f.end, days: f.days } : null },
    proxy: f.proxyMode ? { mode: f.proxyMode, url: f.proxyUrl.trim(), pacUrl: f.pacUrl.trim(), noProxy: f.noProxy.split(/[\n,]/).map((h) => h.trim()).filter(Boolean) } : null,
    ai: { enabled: f.aiEnabled === 'off' ? false : null, allowCloud: f.aiCloud === 'off' ? false : null },
    transfers: { maxUploadKbps: num(f.upKbps), maxDownloadKbps: num(f.downKbps), maxConcurrentRuns: num(f.runs) },
    mounts: {
      allowedProtocols: f.mountProtocols,
      readOnly: f.mountReadOnly === 'on' ? true : null,
      cacheMode: f.mountCache || null,
      maxCacheMb: num(f.mountCacheMb),
      maxCacheAgeHours: num(f.mountCacheHours),
      mountRoot: f.mountRoot.trim() || null,
    },
    workflows: f.workflows.length ? f.workflows.map((w) => ({ id: w.id.trim(), name: w.name.trim(), enabled: w.enabled, rule: parseRule(w.ruleJson) })) : null,
    backupTemplates: f.backupTemplates.length
      ? f.backupTemplates.map((t) => ({ id: t.id.trim(), name: t.name.trim(), description: t.description.trim(), enforced: t.enforced, settings: parseRule(t.settingsJson || '{}') }))
      : null,
  };
}

export default function AppSettingsPage() {
  const { data, error, loading, reload } = useApi<{ settings: ManagedSettings | null; desktops: number; upToDate: number }>('/api/fleet/managed-settings');
  const canWrite = useAuthStore((s) => s.checkPermission)('fleet.write');
  const notify = useToastStore((s) => s.notify);
  const [form, setForm] = useState<Form | null>(null);
  const [seeded, setSeeded] = useState<number | null>(null);
  const [err, setErr] = useState<string | null>(null);

  const version = data?.settings?.version ?? 0;
  if (data && seeded !== version) {
    setSeeded(version);
    setForm(toForm(data.settings));
  }
  if (loading && !data) return <Loading />;
  if (!data || !form) return <ErrorBanner message={error ?? 'Could not load the settings'} />;
  const set = (patch: Partial<Form>) => setForm({ ...form, ...patch });

  const save = async (body: ManagedSettingsBody) => {
    setErr(null);
    try {
      await api('/api/fleet/managed-settings', { method: 'PUT', body });
      notify('Saved; desktops apply it on their next heartbeat', 'success');
      await reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };

  const s = data.settings;
  return (
    <div>
      <PageHeader
        title="App settings"
        description="Settings your organization's desktops must use. Anything on “User's choice” stays with the person using the app."
      />
      {err && <ErrorBanner message={err} />}
      <p className="mb-4 text-sm text-foreground-secondary">
        {s ? `Version ${s.version}, changed ${formatDateTime(s.updatedAt)} by ${s.updatedBy} · applied on ${data.upToDate} of ${data.desktops} desktops` : 'Nothing is managed yet.'}
      </p>
      <fieldset disabled={!canWrite} className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle>Updates</CardTitle>
          </CardHeader>
          <div className="space-y-3">
            <Select label="Update channel" value={form.channel} onChange={(e) => set({ channel: e.target.value as Form['channel'] })} options={[{ value: '', label: "User's choice" }, { value: 'stable', label: 'Stable' }, { value: 'beta', label: 'Beta' }]} />
            <Select label="Automatic update checks" value={form.autoCheck} onChange={(e) => set({ autoCheck: e.target.value as Tri })} options={[{ value: '', label: "User's choice" }, { value: 'on', label: 'On' }, { value: 'off', label: 'Off' }]} />
            <label className="flex items-center gap-2 text-sm">
              <input type="checkbox" checked={form.blackout} onChange={(e) => set({ blackout: e.target.checked })} /> No updates during working hours (blackout window)
            </label>
            {form.blackout && (
              <div className="space-y-2">
                <div className="flex gap-2">
                  <Input label="From" type="time" value={form.start} onChange={(e) => set({ start: e.target.value })} />
                  <Input label="Until" type="time" value={form.end} onChange={(e) => set({ end: e.target.value })} />
                </div>
                <div className="flex flex-wrap gap-1" role="group" aria-label="Blackout days">
                  {DAYS.map((d, i) => (
                    <button
                      key={d}
                      type="button"
                      aria-pressed={form.days.includes(i)}
                      onClick={() => set({ days: form.days.includes(i) ? form.days.filter((x) => x !== i) : [...form.days, i].sort() })}
                      className={`rounded border px-2 py-1 text-xs ${form.days.includes(i) ? 'border-primary bg-primary text-white' : 'border-border'}`}
                    >
                      {d}
                    </button>
                  ))}
                </div>
                <CardDescription>Machine local time. A window that ends before it starts runs overnight. No days selected means every day.</CardDescription>
              </div>
            )}
          </div>
        </Card>
        <Card>
          <CardHeader>
            <CardTitle>Proxy</CardTitle>
          </CardHeader>
          <div className="space-y-3">
            <Select
              label="Proxy"
              value={form.proxyMode}
              onChange={(e) => set({ proxyMode: e.target.value as Form['proxyMode'] })}
              options={[{ value: '', label: "User's choice" }, { value: 'system', label: 'System proxy' }, { value: 'none', label: 'No proxy' }, { value: 'manual', label: 'Manual' }, { value: 'pac', label: 'PAC file' }]}
            />
            {form.proxyMode === 'manual' && <Input label="Proxy URL" placeholder="http://proxy.example.com:3128" value={form.proxyUrl} onChange={(e) => set({ proxyUrl: e.target.value })} />}
            {form.proxyMode === 'pac' && <Input label="PAC file URL" placeholder="https://wpad.example.com/proxy.pac" value={form.pacUrl} onChange={(e) => set({ pacUrl: e.target.value })} />}
            {(form.proxyMode === 'manual' || form.proxyMode === 'pac') && (
              <label className="flex flex-col gap-1.5 text-sm font-medium">
                Bypass (one host per line)
                <textarea className="min-h-20 rounded-md border border-border bg-background p-2 font-mono text-xs" value={form.noProxy} onChange={(e) => set({ noProxy: e.target.value })} />
              </label>
            )}
          </div>
        </Card>
        <Card>
          <CardHeader>
            <CardTitle>AI</CardTitle>
          </CardHeader>
          <div className="space-y-3">
            <Select label="AI features" value={form.aiEnabled} onChange={(e) => set({ aiEnabled: e.target.value as Form['aiEnabled'] })} options={[{ value: '', label: "User's choice" }, { value: 'off', label: 'Off for everyone' }]} />
            <Select label="Cloud models" value={form.aiCloud} onChange={(e) => set({ aiCloud: e.target.value as Form['aiCloud'] })} options={[{ value: '', label: "User's choice" }, { value: 'off', label: 'Local models only' }]} />
          </div>
        </Card>
        <Card>
          <CardHeader>
            <CardTitle>Transfers</CardTitle>
          </CardHeader>
          <div className="space-y-3">
            <Input label="Total upload (KB/s, 0 = unlimited)" type="number" min={0} placeholder="User's choice" value={form.upKbps} onChange={(e) => set({ upKbps: e.target.value })} />
            <Input label="Total download (KB/s, 0 = unlimited)" type="number" min={0} placeholder="User's choice" value={form.downKbps} onChange={(e) => set({ downKbps: e.target.value })} />
            <Input label="Operations at the same time (1–8)" type="number" min={1} max={8} placeholder="User's choice" value={form.runs} onChange={(e) => set({ runs: e.target.value })} />
            <CardDescription>Shared by every backup and restore on a machine. Leave empty for the machine&apos;s own setting.</CardDescription>
          </div>
        </Card>
        <Card>
          <CardHeader>
            <CardTitle>Drive mounts</CardTitle>
          </CardHeader>
          <div className="space-y-3">
            <label className="flex items-center gap-2 text-sm">
              <input type="checkbox" checked={form.mountProtocols !== null} onChange={(e) => set({ mountProtocols: e.target.checked ? ['rclone', 's3'] : null })} /> Only some kinds of drives may be mounted
            </label>
            {form.mountProtocols !== null && (
              <div className="flex flex-wrap gap-1" role="group" aria-label="Allowed mount protocols">
                {MOUNT_PROTOCOLS.map((p) => {
                  const on = form.mountProtocols!.includes(p);
                  return (
                    <button
                      key={p}
                      type="button"
                      aria-pressed={on}
                      onClick={() => set({ mountProtocols: on ? form.mountProtocols!.filter((x) => x !== p) : [...form.mountProtocols!, p] })}
                      className={`rounded border px-2 py-1 text-xs ${on ? 'border-primary bg-primary text-white' : 'border-border'}`}
                    >
                      {p}
                    </button>
                  );
                })}
              </div>
            )}
            <Select label="Read-only" value={form.mountReadOnly} onChange={(e) => set({ mountReadOnly: e.target.value as Form['mountReadOnly'] })} options={[{ value: '', label: "User's choice" }, { value: 'on', label: 'Mount every drive read-only' }]} />
            <Select
              label="Cache mode (cloud drives)"
              value={form.mountCache}
              onChange={(e) => set({ mountCache: e.target.value as Form['mountCache'] })}
              options={[{ value: '', label: "User's choice" }, { value: 'off', label: 'Off' }, { value: 'minimal', label: 'Minimal' }, { value: 'writes', label: 'Writes' }, { value: 'full', label: 'Full (offline use)' }]}
            />
            <Input label="Largest cache per drive (MB)" type="number" min={1} placeholder="User's choice" value={form.mountCacheMb} onChange={(e) => set({ mountCacheMb: e.target.value })} />
            <Input label="Keep cached files (hours)" type="number" min={1} placeholder="User's choice" value={form.mountCacheHours} onChange={(e) => set({ mountCacheHours: e.target.value })} />
            <Input label="Mount points inside" placeholder="User's choice, e.g. ~/Drives" value={form.mountRoot} onChange={(e) => set({ mountRoot: e.target.value })} />
            <CardDescription>The mount profile applies to new mounts; limits cap what people choose, forced options replace it.</CardDescription>
          </div>
        </Card>
        <Card className="lg:col-span-2">
          <CardHeader>
            <CardTitle>Organization workflows</CardTitle>
            <CardDescription>
              Automation rules every desktop runs and nobody can edit there. In the desktop app, open a rule and choose “Copy as JSON”, then paste it here. Rules that run programs are refused.
            </CardDescription>
          </CardHeader>
          <div className="space-y-4">
            {form.workflows.map((w, i) => {
              const patch = (p: Partial<WorkflowForm>) => set({ workflows: form.workflows.map((x, j) => (j === i ? { ...x, ...p } : x)) });
              return (
                <div key={i} role="group" aria-label={`Workflow ${i + 1}`} className="space-y-2 rounded border border-border p-3">
                  <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
                    <Input label="Name" value={w.name} onChange={(e) => patch({ name: e.target.value })} />
                    <Input label="Id" placeholder="From the name" value={w.id} onChange={(e) => patch({ id: e.target.value })} />
                  </div>
                  <label className="flex flex-col gap-1 text-sm">
                    Rule (JSON)
                    <textarea className="min-h-32 rounded border border-border bg-background p-2 font-mono text-xs" spellCheck={false} value={w.ruleJson} onChange={(e) => patch({ ruleJson: e.target.value })} />
                  </label>
                  <div className="flex items-center justify-between">
                    <label className="flex items-center gap-2 text-sm">
                      <input type="checkbox" checked={w.enabled} onChange={(e) => patch({ enabled: e.target.checked })} /> Enabled
                    </label>
                    <Button variant="outline" onClick={() => set({ workflows: form.workflows.filter((_, j) => j !== i) })}>
                      Remove
                    </Button>
                  </div>
                </div>
              );
            })}
            <Button variant="outline" onClick={() => set({ workflows: [...form.workflows, { id: '', name: '', enabled: true, ruleJson: '' }] })}>
              Add workflow
            </Button>
          </div>
        </Card>
        <Card className="lg:col-span-2">
          <CardHeader>
            <CardTitle>Backup policy templates</CardTitle>
            <CardDescription>
              Templates backup jobs on every desktop can be based on; an enforced template applies to every job and wins over its own settings. In the desktop app, export a template (Settings → Policy templates) and paste its “settings” here. Templates that set scripts are refused.
            </CardDescription>
          </CardHeader>
          <div className="space-y-4">
            {form.backupTemplates.map((t, i) => {
              const patch = (p: Partial<BackupTemplateForm>) => set({ backupTemplates: form.backupTemplates.map((x, j) => (j === i ? { ...x, ...p } : x)) });
              return (
                <div key={i} role="group" aria-label={`Backup template ${i + 1}`} className="space-y-2 rounded border border-border p-3">
                  <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
                    <Input label="Name" value={t.name} onChange={(e) => patch({ name: e.target.value })} />
                    <Input label="Id" placeholder="From the name" value={t.id} onChange={(e) => patch({ id: e.target.value })} />
                  </div>
                  <Input label="Description" value={t.description} onChange={(e) => patch({ description: e.target.value })} />
                  <label className="flex flex-col gap-1 text-sm">
                    Settings (JSON)
                    <textarea className="min-h-32 rounded border border-border bg-background p-2 font-mono text-xs" spellCheck={false} value={t.settingsJson} onChange={(e) => patch({ settingsJson: e.target.value })} />
                  </label>
                  <div className="flex items-center justify-between">
                    <label className="flex items-center gap-2 text-sm">
                      <input type="checkbox" checked={t.enforced} onChange={(e) => patch({ enforced: e.target.checked })} /> Enforced (applies to every job)
                    </label>
                    <Button variant="outline" onClick={() => set({ backupTemplates: form.backupTemplates.filter((_, j) => j !== i) })}>
                      Remove
                    </Button>
                  </div>
                </div>
              );
            })}
            <Button variant="outline" onClick={() => set({ backupTemplates: [...form.backupTemplates, { id: '', name: '', description: '', enforced: false, settingsJson: '{}' }] })}>
              Add backup template
            </Button>
          </div>
        </Card>
      </fieldset>
      {canWrite && (
        <div className="mt-4 flex gap-2">
          <Button onClick={() => void save(toBody(form))}>Save</Button>
          {s && (
            <Button variant="outline" onClick={() => confirm('Stop managing app settings? Desktops give the settings back to their users.') && void save({ updates: null, proxy: null, ai: null, transfers: null, mounts: null, workflows: null, backupTemplates: null })}>
              Stop managing
            </Button>
          )}
        </div>
      )}
    </div>
  );
}
