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
  };
}

function toBody(f: Form): ManagedSettingsBody {
  return {
    updates: { channel: f.channel || null, autoCheck: fromTri(f.autoCheck), blackout: f.blackout ? { start: f.start, end: f.end, days: f.days } : null },
    proxy: f.proxyMode ? { mode: f.proxyMode, url: f.proxyUrl.trim(), pacUrl: f.pacUrl.trim(), noProxy: f.noProxy.split(/[\n,]/).map((h) => h.trim()).filter(Boolean) } : null,
    ai: { enabled: f.aiEnabled === 'off' ? false : null, allowCloud: f.aiCloud === 'off' ? false : null },
    transfers: { maxUploadKbps: num(f.upKbps), maxDownloadKbps: num(f.downKbps), maxConcurrentRuns: num(f.runs) },
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
      </fieldset>
      {canWrite && (
        <div className="mt-4 flex gap-2">
          <Button onClick={() => void save(toBody(form))}>Save</Button>
          {s && (
            <Button variant="outline" onClick={() => confirm('Stop managing app settings? Desktops give the settings back to their users.') && void save({ updates: null, proxy: null, ai: null, transfers: null })}>
              Stop managing
            </Button>
          )}
        </div>
      )}
    </div>
  );
}
