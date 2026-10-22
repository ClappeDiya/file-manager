'use client';

/** Settings → Retention: how long each log is kept (console-wide). */
import React, { useState } from 'react';
import { PageHeader } from '@/components/layout/page-header';
import { Card } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { ErrorBanner, Loading } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { useToastStore } from '@/lib/stores/toast-store';
import { formatDateTime } from '@/lib/utils/format';
import type { LogType, LogTypeKey, RetentionSettings } from '@/lib/retention';

interface Data {
  settings: RetentionSettings;
  types: Record<LogTypeKey, LogType>;
  canEdit: boolean;
  lastAuditPrune: { throughSequence: number; pruned: number; createdAt: string } | null;
  auditVerifies: boolean;
  auditReason: string | null;
}

export default function RetentionPage() {
  const { data, error, loading, reload } = useApi<Data>('/api/retention');
  const notify = useToastStore((s) => s.notify);
  const [draft, setDraft] = useState<Partial<Record<LogTypeKey, string>>>({});
  const [err, setErr] = useState<string | null>(null);
  if (loading && !data) return <Loading />;
  if (!data) return <ErrorBanner message={error ?? 'Could not load retention'} />;

  const save = async (applyNow: boolean) => {
    setErr(null);
    try {
      const patch = Object.fromEntries(Object.entries(draft).map(([k, v]) => [k, Number(v)]));
      const r = await api<{ removed: Record<string, number> }>('/api/retention', { method: 'PUT', body: { ...patch, applyNow } });
      setDraft({});
      notify(
        applyNow
          ? `Applied: ${
              Object.entries(r.removed)
                .map(([k, n]) => `${data.types[k as LogTypeKey]?.label ?? k} ${n}`)
                .join(', ') || 'nothing to remove'
            }`
          : 'Retention saved',
        'success',
      );
      await reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };

  return (
    <div>
      <PageHeader title="Retention" description="How long the console keeps each log. Applies to every organization; cleanup runs with the periodic jobs." />
      <ErrorBanner message={err} />
      {!data.canEdit && <p className="mb-3 text-sm text-foreground-secondary">Only owners of a top-level organization can change these.</p>}
      <Card className="mb-4 text-sm">
        Audit log: {data.auditVerifies ? 'the chain verifies' : `the chain does NOT verify (${data.auditReason}) — it will not be pruned`}.{' '}
        {data.lastAuditPrune ? `Last pruned ${formatDateTime(data.lastAuditPrune.createdAt)}: ${data.lastAuditPrune.pruned} entries up to #${data.lastAuditPrune.throughSequence} (a signed checkpoint).` : 'Never pruned.'}
      </Card>
      <fieldset disabled={!data.canEdit} className="grid grid-cols-1 gap-3 md:grid-cols-2">
        {(Object.keys(data.types) as LogTypeKey[]).map((k) => {
          const t = data.types[k];
          return (
            <Card key={k}>
              <Input
                label={`${t.label} (days${t.allowForever ? ', 0 = forever' : ''}; ${t.min}–${t.max})`}
                type="number"
                value={draft[k] ?? String(data.settings[k])}
                onChange={(e) => setDraft({ ...draft, [k]: e.target.value })}
              />
              <p className="mt-1 text-xs text-foreground-tertiary">{t.description}</p>
            </Card>
          );
        })}
      </fieldset>
      {data.canEdit && (
        <div className="mt-4 flex gap-2">
          <Button disabled={!Object.keys(draft).length} onClick={() => void save(false)}>
            Save
          </Button>
          <Button variant="outline" onClick={() => void save(true)}>
            Save and clean up now
          </Button>
        </div>
      )}
    </div>
  );
}
