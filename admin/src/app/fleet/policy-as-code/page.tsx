'use client';

/**
 * Policy as code: download the organization's fleet policies, app settings
 * and alert rules as one JSON document, and apply a document after
 * reviewing what it would change.
 */
import React, { useState } from 'react';
import { PageHeader } from '@/components/layout/page-header';
import { Card, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { ErrorBanner } from '@/components/fleet/badges';
import { api, errorText } from '@/lib/api-client';
import { useAuthStore } from '@/lib/stores/auth-store';
import { useToastStore } from '@/lib/stores/toast-store';
import type { Plan } from '@/lib/policy-as-code';

const KIND: Record<string, string> = { managed_settings: 'App settings', fleet_policy: 'Fleet policy', alert_rule: 'Alert rule' };
const TONE: Record<string, string> = { create: 'text-green-600', update: 'text-amber-600', delete: 'text-red-600', unchanged: 'text-foreground-secondary' };

export default function PolicyAsCodePage() {
  const check = useAuthStore((s) => s.checkPermission);
  const canApply = check('fleet.write') && check('alerts.write');
  const notify = useToastStore((s) => s.notify);
  const [text, setText] = useState('');
  const [prune, setPrune] = useState(false);
  const [plan, setPlan] = useState<Plan | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const parse = (): unknown | null => {
    try {
      return JSON.parse(text);
    } catch (e) {
      setErr(`The document is not valid JSON: ${(e as Error).message}`);
      return null;
    }
  };
  const run = async (apply: boolean) => {
    setErr(null);
    const document = parse();
    if (!document) return;
    setBusy(true);
    try {
      const r = apply
        ? await api<{ plan: Plan }>('/api/fleet/policy-as-code', { method: 'PUT', body: { document, prune } })
        : await api<{ plan: Plan }>('/api/fleet/policy-as-code/plan', { method: 'POST', body: { document, prune } });
      setPlan(r.plan);
      if (apply) notify(`Applied: ${r.plan.summary.create} created, ${r.plan.summary.update} updated, ${r.plan.summary.delete} deleted`, 'success');
    } catch (e) {
      setErr(errorText(e));
      setPlan(null);
    } finally {
      setBusy(false);
    }
  };
  const exportDoc = async () => {
    setErr(null);
    try {
      const doc = await api<unknown>('/api/fleet/policy-as-code');
      const json = JSON.stringify(doc, null, 2);
      setText(json);
      const url = URL.createObjectURL(new Blob([json], { type: 'application/json' }));
      const a = document.createElement('a');
      a.href = url;
      a.download = 'ufop-policies.json';
      a.click();
      URL.revokeObjectURL(url);
    } catch (e) {
      setErr(errorText(e));
    }
  };
  const changed = plan ? plan.changes.filter((c) => c.action !== 'unchanged') : [];

  return (
    <div>
      <PageHeader title="Policy as code" description="Fleet policies, app settings and alert rules as one document: keep it in version control, review changes, apply it here or to another organization." />
      {err && <ErrorBanner message={err} />}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle>Document</CardTitle>
            <CardDescription>Items match by name. Channels and central credentials are referenced by name (`central-name:…`); secrets are never included.</CardDescription>
          </CardHeader>
          <div className="space-y-3">
            <Button variant="outline" onClick={() => void exportDoc()}>
              Export this organization
            </Button>
            <label className="block text-sm">
              Paste or edit a document
              <textarea aria-label="Policy document" className="mt-1 min-h-80 w-full rounded border border-border bg-background p-2 font-mono text-xs" spellCheck={false} value={text} onChange={(e) => { setText(e.target.value); setPlan(null); }} />
            </label>
            <label className="flex items-center gap-2 text-sm">
              <input type="checkbox" checked={prune} onChange={(e) => { setPrune(e.target.checked); setPlan(null); }} /> Delete items the document doesn&apos;t list
            </label>
            <div className="flex gap-2">
              <Button variant="outline" disabled={busy || !text.trim()} onClick={() => void run(false)}>
                Review changes
              </Button>
              {canApply && (
                <Button disabled={busy || !plan || changed.length === 0} onClick={() => confirm(`Apply ${changed.length} change(s) to this organization?`) && void run(true)}>
                  Apply
                </Button>
              )}
            </div>
          </div>
        </Card>
        <Card>
          <CardHeader>
            <CardTitle>Changes</CardTitle>
            <CardDescription>{plan ? `${plan.summary.create} to create, ${plan.summary.update} to update, ${plan.summary.delete} to delete, ${plan.summary.unchanged} unchanged` : 'Review a document to see what applying it would change.'}</CardDescription>
          </CardHeader>
          {plan && (
            <ul className="space-y-1 text-sm" aria-label="Planned changes">
              {plan.changes.map((c) => (
                <li key={`${c.kind}:${c.name}`} className="flex justify-between gap-2">
                  <span>
                    {KIND[c.kind]} <b>{c.name}</b>
                    {c.fields && <span className="text-foreground-secondary"> · {c.fields.join(', ')}</span>}
                  </span>
                  <span className={TONE[c.action]}>{c.action}</span>
                </li>
              ))}
            </ul>
          )}
        </Card>
      </div>
    </div>
  );
}
