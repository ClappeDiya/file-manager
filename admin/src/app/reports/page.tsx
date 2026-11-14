'use client';

/** Reports: the catalog (run and download), schedules that email reports, and recent runs. */
import React, { useState } from 'react';
import { FileBarChart } from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { Badge } from '@/components/ui/badge';
import { EmptyState } from '@/components/ui/empty-state';
import { Dialog, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { ErrorBanner, Loading } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { useAuthStore } from '@/lib/stores/auth-store';
import { formatDateTime, getDisplayTimeZone } from '@/lib/utils/format';
import type { ReportRun, ReportSchedule } from '@/lib/report-runner';
import type { Channel } from '@/lib/db/repos/channels';

interface CatalogEntry {
  id: string;
  name: string;
  description: string;
  category: string;
  defaultDays: number;
}

const WEEKDAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];
const download = (runId: string, format: string) => window.open(`/api/reports/runs/${runId}?format=${format}`, '_blank');

interface Draft {
  reportId: string;
  name: string;
  days: string;
  frequency: 'daily' | 'weekly' | 'monthly';
  hour: string;
  weekday: string;
  dayOfMonth: string;
  timeZone: string;
  format: 'csv' | 'json';
  channelIds: string[];
}

export default function ReportsPage() {
  const check = useAuthStore((s) => s.checkPermission);
  const canSchedule = check('alerts.write');
  const { data, error, loading, reload } = useApi<{ catalog: CatalogEntry[]; schedules: ReportSchedule[]; runs: ReportRun[] }>('/api/reports', 30_000);
  const channels = useApi<{ channels: Channel[] }>(check('alerts.read') ? '/api/alerts/channels' : null);
  const [days, setDays] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [draft, setDraft] = useState<Draft | null>(null);
  const names = new Map((data?.catalog ?? []).map((r) => [r.id, r.name]));

  const act = async (key: string, f: () => Promise<unknown>) => {
    setBusy(key);
    setErr(null);
    try {
      await f();
      await reload();
    } catch (e) {
      setErr(errorText(e));
    } finally {
      setBusy(null);
    }
  };
  const runNow = (r: CatalogEntry) =>
    act(r.id, async () => {
      const res = await api<{ run: ReportRun }>('/api/reports/run', { body: { reportId: r.id, days: Number(days[r.id] || r.defaultDays) } });
      download(res.run.id, 'csv');
    });
  const saveSchedule = () =>
    act('schedule', async () => {
      if (!draft) return;
      await api('/api/reports/schedules', {
        body: {
          reportId: draft.reportId,
          name: draft.name || undefined,
          days: Number(draft.days),
          frequency: draft.frequency,
          hour: Number(draft.hour),
          weekday: Number(draft.weekday),
          dayOfMonth: Number(draft.dayOfMonth),
          timeZone: draft.timeZone,
          format: draft.format,
          channelIds: draft.channelIds,
        },
      });
      setDraft(null);
    });
  const when = (s: ReportSchedule) =>
    `${s.frequency === 'daily' ? 'Daily' : s.frequency === 'weekly' ? `Every ${WEEKDAYS[s.weekday ?? 1]}` : `Monthly on day ${s.dayOfMonth}`} at ${String(s.hour).padStart(2, '0')}:00 ${s.timeZone}`;

  return (
    <div>
      <PageHeader title="Reports" description="Run reports on demand, download them as CSV or JSON, or email them on a schedule." />
      <ErrorBanner message={err ?? error} />
      {loading && !data ? (
        <Loading />
      ) : (
        <div className="space-y-6">
          <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
            {data?.catalog.map((r) => (
              <Card key={r.id}>
                <CardHeader className="flex-col items-start gap-1">
                  <CardTitle>{r.name}</CardTitle>
                  <CardDescription>{r.description}</CardDescription>
                </CardHeader>
                <div className="flex items-end gap-2">
                  <Input label="Days" type="number" min={1} max={365} className="w-24" value={days[r.id] ?? String(r.defaultDays)} onChange={(e) => setDays({ ...days, [r.id]: e.target.value })} />
                  <Button size="sm" disabled={busy === r.id} onClick={() => void runNow(r)}>
                    Run and download
                  </Button>
                  {canSchedule && (
                    <Button
                      size="sm"
                      variant="outline"
                      onClick={() =>
                        setDraft({ reportId: r.id, name: '', days: String(r.defaultDays), frequency: 'weekly', hour: '7', weekday: '1', dayOfMonth: '1', timeZone: getDisplayTimeZone(), format: 'csv', channelIds: [] })
                      }
                    >
                      Schedule
                    </Button>
                  )}
                </div>
              </Card>
            ))}
          </div>

          <Card>
            <CardTitle className="mb-3">Schedules</CardTitle>
            {!data?.schedules.length ? (
              <EmptyState icon={FileBarChart} title="No scheduled reports" description="Schedule a report to email it to a channel daily, weekly or monthly." />
            ) : (
              <ul className="divide-y divide-border text-sm">
                {data.schedules.map((s) => (
                  <li key={s.id} className="flex flex-wrap items-center justify-between gap-2 py-2">
                    <span>
                      <strong>{s.name}</strong>{' '}
                      <span className="text-xs text-foreground-tertiary">
                        · {when(s)} · last {s.days} days · {s.format.toUpperCase()} · {s.channelIds.length} channel(s)
                      </span>
                      <span className="block text-xs text-foreground-tertiary">{s.enabled ? `Next: ${s.nextRunAt ? formatDateTime(s.nextRunAt) : '—'}` : 'Paused'}</span>
                    </span>
                    {canSchedule && (
                      <span className="flex gap-1">
                        <Button size="sm" variant="ghost" disabled={busy === s.id} onClick={() => void act(s.id, () => api(`/api/reports/schedules/${s.id}/run`, { body: {} }))}>
                          Send now
                        </Button>
                        <Button size="sm" variant="ghost" onClick={() => void act(s.id, () => api(`/api/reports/schedules/${s.id}`, { method: 'PATCH', body: { enabled: !s.enabled } }))}>
                          {s.enabled ? 'Pause' : 'Resume'}
                        </Button>
                        <Button size="sm" variant="ghost" onClick={() => void act(s.id, () => api(`/api/reports/schedules/${s.id}`, { method: 'DELETE' }))}>
                          Delete
                        </Button>
                      </span>
                    )}
                  </li>
                ))}
              </ul>
            )}
          </Card>

          <Card>
            <CardTitle className="mb-3">Recent runs</CardTitle>
            <ul className="divide-y divide-border text-sm">
              {data?.runs.map((r) => (
                <li key={r.id} className="flex flex-wrap items-center justify-between gap-2 py-2">
                  <span>
                    <Badge variant={r.status === 'succeeded' ? 'success' : r.status === 'failed' ? 'error' : 'default'}>{r.status}</Badge> {names.get(r.reportId) ?? r.reportId}{' '}
                    <span className="text-xs text-foreground-tertiary">
                      · {formatDateTime(r.startedAt)} · {r.requestedBy} · {r.error ?? r.summary}
                      {r.delivery.length > 0 && ` · sent to ${r.delivery.filter((d) => d.ok).length}/${r.delivery.length}`}
                    </span>
                    {r.delivery
                      .filter((d) => !d.ok)
                      .map((d) => (
                        <span key={d.channelId} className="block text-xs text-error">
                          {d.channelId}: {d.error}
                        </span>
                      ))}
                  </span>
                  {r.status === 'succeeded' && (
                    <span className="flex gap-1">
                      {['csv', 'json'].map((f) => (
                        <Button key={f} size="sm" variant="ghost" onClick={() => download(r.id, f)}>
                          {f.toUpperCase()}
                        </Button>
                      ))}
                    </span>
                  )}
                </li>
              ))}
              {data && !data.runs.length && <li className="py-2 text-foreground-tertiary">No runs yet.</li>}
            </ul>
          </Card>
        </div>
      )}

      <Dialog open={!!draft} onClose={() => setDraft(null)}>
        <DialogHeader>
          <DialogTitle>Schedule “{draft ? names.get(draft.reportId) : ''}”</DialogTitle>
          <DialogDescription>Emails get the report attached (and a text table); chat and webhook channels get the summary.</DialogDescription>
        </DialogHeader>
        {draft && (
          <div className="space-y-3">
            <Input label="Name (optional)" value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} />
            <div className="grid grid-cols-2 gap-2">
              <Select
                label="How often"
                options={[
                  { value: 'daily', label: 'Daily' },
                  { value: 'weekly', label: 'Weekly' },
                  { value: 'monthly', label: 'Monthly' },
                ]}
                value={draft.frequency}
                onChange={(e) => setDraft({ ...draft, frequency: e.target.value as Draft['frequency'] })}
              />
              {draft.frequency === 'weekly' && (
                <Select label="Day" options={WEEKDAYS.map((d, i) => ({ value: String(i), label: d }))} value={draft.weekday} onChange={(e) => setDraft({ ...draft, weekday: e.target.value })} />
              )}
              {draft.frequency === 'monthly' && <Input label="Day of month (1–28)" type="number" min={1} max={28} value={draft.dayOfMonth} onChange={(e) => setDraft({ ...draft, dayOfMonth: e.target.value })} />}
              <Input label="Hour (0–23)" type="number" min={0} max={23} value={draft.hour} onChange={(e) => setDraft({ ...draft, hour: e.target.value })} />
              <Input label="Time zone" value={draft.timeZone} onChange={(e) => setDraft({ ...draft, timeZone: e.target.value })} />
              <Input label="Days covered" type="number" min={1} max={365} value={draft.days} onChange={(e) => setDraft({ ...draft, days: e.target.value })} />
              <Select
                label="Attachment"
                options={[
                  { value: 'csv', label: 'CSV' },
                  { value: 'json', label: 'JSON' },
                ]}
                value={draft.format}
                onChange={(e) => setDraft({ ...draft, format: e.target.value as Draft['format'] })}
              />
            </div>
            <fieldset className="space-y-1 text-sm">
              <legend className="mb-1 font-medium">Send to</legend>
              {channels.data?.channels.map((c) => (
                <label key={c.id} className="flex items-center gap-2">
                  <input
                    type="checkbox"
                    checked={draft.channelIds.includes(c.id)}
                    onChange={(e) => setDraft({ ...draft, channelIds: e.target.checked ? [...draft.channelIds, c.id] : draft.channelIds.filter((x) => x !== c.id) })}
                  />
                  {c.name} <span className="text-xs text-foreground-tertiary">({c.kind})</span>
                </label>
              ))}
              {!channels.data?.channels.length && <p className="text-foreground-tertiary">Add a notification channel under Alerts first.</p>}
            </fieldset>
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => setDraft(null)}>
            Cancel
          </Button>
          <Button disabled={!draft?.channelIds.length || busy === 'schedule'} onClick={() => void saveSchedule()}>
            Save
          </Button>
        </DialogFooter>
      </Dialog>
    </div>
  );
}
