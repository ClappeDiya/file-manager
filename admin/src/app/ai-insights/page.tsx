'use client';

import React, { useEffect, useState } from 'react';
import { Sparkles } from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { EmptyState } from '@/components/ui/empty-state';
import { ErrorBanner, Loading } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { useAuthStore } from '@/lib/stores/auth-store';
import { formatDateTime } from '@/lib/utils/format';
import type { AiReport, AiSettings } from '@/lib/ai';
import type { Channel } from '@/lib/db/repos/channels';
import { TranscriptDialog } from '@/components/ai/transcript-dialog';

export default function AiInsightsPage() {
  const check = useAuthStore((s) => s.checkPermission);
  const canConfigure = check('ai.write');
  const settings = useApi<{ settings: AiSettings }>('/api/ai/settings');
  const reports = useApi<{ reports: AiReport[] }>('/api/ai/analyze');
  const machines = useApi<{ machines: { id: string; name: string }[] }>('/api/fleet/machines');
  const channels = useApi<{ channels: Channel[] }>(check('alerts.read') ? '/api/alerts/channels' : null);
  const [draft, setDraft] = useState<(Omit<AiSettings, 'orgId' | 'hasApiKey' | 'lastRunAt'> & { apiKey: string }) | null>(null);
  const [machineId, setMachineId] = useState('');
  const [question, setQuestion] = useState('');
  const [days, setDays] = useState('30');
  const [busy, setBusy] = useState(false);
  const [transcriptOf, setTranscriptOf] = useState<string | null>(null);
  const canAudit = check('audit.read');
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    const s = settings.data?.settings;
    if (s && !draft) setDraft({ enabled: s.enabled, model: s.model, instructions: s.instructions, schedule: s.schedule, channelIds: s.channelIds, apiBase: s.apiBase, apiKey: '' });
  }, [settings.data, draft]);

  const save = async () => {
    if (!draft) return;
    setErr(null);
    const { apiKey, ...rest } = draft;
    try {
      await api('/api/ai/settings', { method: 'PUT', body: apiKey ? { ...rest, apiKey } : rest });
      setDraft({ ...draft, apiKey: '' });
      await settings.reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };

  const analyze = async () => {
    setErr(null);
    setBusy(true);
    try {
      await api('/api/ai/analyze', { body: { machineId: machineId || undefined, question: question || undefined, days: Number(days) } });
      setQuestion('');
    } catch (e) {
      setErr(errorText(e));
    } finally {
      setBusy(false);
      await reports.reload();
    }
  };

  const s = settings.data?.settings;
  return (
    <div>
      <PageHeader title="AI insights" description="Ask Claude to analyze backup reports, explain failures and summarize fleet health." />
      <ErrorBanner message={err ?? settings.error ?? reports.error} />
      <TranscriptDialog reportId={transcriptOf} onClose={() => setTranscriptOf(null)} />
      {!s || !draft ? (
        <Loading />
      ) : (
        <div className="grid grid-cols-1 gap-4 xl:grid-cols-3">
          <div className="space-y-4 xl:col-span-2">
            <Card>
              <CardHeader className="flex-col items-start gap-1">
                <CardTitle>Analyze</CardTitle>
                <CardDescription>The machines&apos; reports and alerts for the chosen period are sent to the model. File names and paths are not included.</CardDescription>
              </CardHeader>
              {!s.enabled || !s.hasApiKey ? (
                <p className="text-sm text-foreground-secondary">Turn AI insights on and add an API key{canConfigure ? ' in the settings.' : ' (ask an administrator).'}</p>
              ) : (
                <div className="space-y-3">
                  <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                    <Select label="Scope" value={machineId} onChange={(e) => setMachineId(e.target.value)} placeholder="Whole fleet" options={(machines.data?.machines ?? []).map((m) => ({ value: m.id, label: m.name }))} />
                    <Select label="Period" value={days} onChange={(e) => setDays(e.target.value)} options={[{ value: '7', label: '7 days' }, { value: '30', label: '30 days' }, { value: '90', label: '90 days' }]} />
                  </div>
                  <Input label="Question (optional)" placeholder="Why did last night's backups fail?" value={question} onChange={(e) => setQuestion(e.target.value)} />
                  <Button onClick={() => void analyze()} loading={busy}>
                    <Sparkles size={14} className="mr-2" aria-hidden="true" />
                    Analyze
                  </Button>
                </div>
              )}
            </Card>
            {reports.loading && !reports.data ? (
              <Loading />
            ) : !reports.data?.reports.length ? (
              <Card>
                <EmptyState icon={Sparkles} title="No analyses yet" description="Run an analysis, or set a daily or weekly summary." />
              </Card>
            ) : (
              reports.data.reports.map((r) => (
                <Card key={r.id}>
                  <div className="mb-2 flex flex-wrap items-baseline justify-between gap-2">
                    <p className="text-sm font-medium">
                      {r.kind === 'summary' ? 'Scheduled summary' : 'Analysis'}
                      {typeof r.scope.question === 'string' && r.scope.question ? `: ${r.scope.question}` : ''}
                    </p>
                    <p className="text-xs text-foreground-tertiary">
                      {formatDateTime(r.createdAt)} · {r.createdBy} · {r.model} · {r.inputTokens + r.outputTokens} tokens
                      {canAudit && (
                        <button className="ml-2 underline" onClick={() => setTranscriptOf(r.id)}>
                          Transcript
                        </button>
                      )}
                    </p>
                  </div>
                  {r.error ? <p className="text-sm text-error">{r.error}</p> : <div className="whitespace-pre-wrap text-sm leading-relaxed text-foreground">{r.content}</div>}
                </Card>
              ))
            )}
          </div>
          <Card className="h-fit">
            <CardTitle className="mb-3">Settings</CardTitle>
            <fieldset disabled={!canConfigure} className="space-y-3">
              <label className="flex items-center gap-2 text-sm">
                <input type="checkbox" checked={draft.enabled} onChange={(e) => setDraft({ ...draft, enabled: e.target.checked })} />
                Enable AI insights for this organization
              </label>
              <Input label="Anthropic API key" type="password" autoComplete="off" placeholder={s.hasApiKey ? '•••••• (stored)' : 'sk-ant-…'} value={draft.apiKey} onChange={(e) => setDraft({ ...draft, apiKey: e.target.value })} hint="Stored encrypted; never shown again." />
              <Input label="Model" value={draft.model} onChange={(e) => setDraft({ ...draft, model: e.target.value })} />
              <Input label="API base URL (optional)" placeholder="https://api.anthropic.com" value={draft.apiBase} onChange={(e) => setDraft({ ...draft, apiBase: e.target.value })} />
              <Select label="Scheduled summary" value={draft.schedule} onChange={(e) => setDraft({ ...draft, schedule: e.target.value as AiSettings['schedule'] })} options={[{ value: 'off', label: 'Off' }, { value: 'daily', label: 'Daily' }, { value: 'weekly', label: 'Weekly' }]} />
              {draft.schedule !== 'off' && !!channels.data?.channels.length && (
                <fieldset>
                  <legend className="mb-1 text-sm font-medium">Send summaries to</legend>
                  {channels.data.channels.map((c) => (
                    <label key={c.id} className="mr-4 inline-flex items-center gap-2 text-sm">
                      <input type="checkbox" checked={draft.channelIds.includes(c.id)} onChange={(e) => setDraft({ ...draft, channelIds: e.target.checked ? [...draft.channelIds, c.id] : draft.channelIds.filter((x) => x !== c.id) })} />
                      {c.name}
                    </label>
                  ))}
                </fieldset>
              )}
              <label className="flex flex-col gap-1.5 text-sm font-medium text-foreground">
                Extra instructions
                <textarea className="min-h-20 rounded-md border border-border bg-background p-2 text-sm" value={draft.instructions} onChange={(e) => setDraft({ ...draft, instructions: e.target.value })} placeholder="Focus on finance machines; our RPO is 24 hours." />
              </label>
              {s.lastRunAt && <p className="text-xs text-foreground-tertiary">Last summary {formatDateTime(s.lastRunAt)}</p>}
              {canConfigure && <Button onClick={() => void save()}>Save settings</Button>}
            </fieldset>
          </Card>
        </div>
      )}
    </div>
  );
}
