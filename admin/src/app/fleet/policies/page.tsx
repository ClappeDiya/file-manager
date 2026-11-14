'use client';

import React, { useState } from 'react';
import { ClipboardList, Plus } from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { EmptyState } from '@/components/ui/empty-state';
import { Dialog, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { ErrorBanner, Loading, TagChip } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { useAuthStore } from '@/lib/stores/auth-store';
import { formatRelativeTime } from '@/lib/utils/format';
import type { FleetPolicy, CentralCredential, InheritedPolicy } from '@/lib/db/repos/fleet-policies';

interface Compliance {
  targeted: number;
  upToDate: number;
  pending: { id: string; name: string; applied: number | null }[];
}
type Row = FleetPolicy & Compliance;
type InheritedRow = InheritedPolicy & Compliance;

const TEMPLATE = {
  name: 'Documents',
  sources: ['~/Documents'],
  destination: { type: 'cloud', config: { provider: 's3', bucket: 'company-backups', prefix: 'docs/{hostname}', region: 'eu-west-1' }, credential_id: 'central:<credential id>' },
  encrypted: true,
  passphraseCredentialId: '<credential id>',
  schedule: { type: 'interval', minutes: 1440 },
  retention: { type: 'smart' },
};

interface Draft {
  id: string | null;
  name: string;
  description: string;
  all: boolean;
  tags: string;
  enabled: boolean;
  inherit: boolean;
  locked: boolean;
  job: string;
}

const empty: Draft = { id: null, name: '', description: '', all: false, tags: '', enabled: true, inherit: false, locked: false, job: JSON.stringify(TEMPLATE, null, 2) };

export default function FleetPoliciesPage() {
  const { data, error, loading, reload } = useApi<{ policies: Row[]; inherited: InheritedRow[] }>('/api/fleet/policies');
  const creds = useApi<{ credentials: CentralCredential[] }>('/api/fleet/credentials');
  const canWrite = useAuthStore((s) => s.checkPermission('fleet.write'));
  const [draft, setDraft] = useState<Draft | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [formErr, setFormErr] = useState<string | null>(null);

  const save = async () => {
    if (!draft) return;
    setFormErr(null);
    let job: Record<string, unknown>;
    try {
      job = JSON.parse(draft.job);
    } catch {
      setFormErr('The job template is not valid JSON');
      return;
    }
    const body = { name: draft.name, description: draft.description, enabled: draft.enabled, inherit: draft.inherit, locked: draft.inherit && draft.locked, target: { all: draft.all, tags: draft.tags.split(',').map((t) => t.trim()).filter(Boolean), machineIds: [] }, job };
    try {
      await api(draft.id ? `/api/fleet/policies/${draft.id}` : '/api/fleet/policies', { method: draft.id ? 'PUT' : 'POST', body });
      setDraft(null);
      await reload();
    } catch (e) {
      setFormErr(errorText(e));
    }
  };

  const optOut = async (i: InheritedRow, value: boolean) => {
    setErr(null);
    try {
      await api(`/api/fleet/policies/${i.policy.id}/opt-out`, { body: { optOut: value } });
      await reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };

  const remove = async (p: Row) => {
    if (!confirm(`Delete policy "${p.name}"? Machines remove the managed job on their next check-in.`)) return;
    try {
      await api(`/api/fleet/policies/${p.id}`, { method: 'DELETE' });
      await reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };

  return (
    <div>
      <PageHeader
        title="Fleet policies"
        description="Backup jobs defined here are created, updated and removed on the targeted machines automatically."
        actions={
          canWrite && (
            <Button onClick={() => setDraft(empty)}>
              <Plus size={14} className="mr-2" aria-hidden="true" />
              New policy
            </Button>
          )
        }
      />
      <ErrorBanner message={err ?? error} />
      {loading && !data ? (
        <Loading />
      ) : !data?.policies.length && !data?.inherited.length ? (
        <Card>
          <EmptyState icon={ClipboardList} title="No fleet policies" description="Create a policy to push the same backup job to every machine with a tag." />
        </Card>
      ) : (
        <div className="space-y-3">
          {data?.policies.map((p) => (
            <Card key={p.id}>
              <div className="flex flex-col gap-3 md:flex-row md:items-start md:justify-between">
                <div>
                  <h3 className="font-semibold text-foreground">
                    {p.name} <span className="text-xs font-normal text-foreground-tertiary">v{p.version}</span>
                    {!p.enabled && <span className="ml-2 text-xs text-warning">disabled</span>}
                    {p.inherit && <span className="ml-2 text-xs text-info">{p.locked ? 'locked for child organizations' : 'inherited by child organizations'}</span>}
                  </h3>
                  {p.description && <p className="text-sm text-foreground-secondary">{p.description}</p>}
                  <div className="mt-2 flex flex-wrap items-center gap-1 text-xs text-foreground-secondary">
                    Targets: {p.target.all ? 'every machine' : p.target.tags.length ? p.target.tags.map((t) => <TagChip key={t} tag={t} />) : 'nothing yet'}
                  </div>
                  <p className="mt-1 text-xs text-foreground-tertiary">
                    Job “{String(p.job.name)}” · updated {formatRelativeTime(p.updatedAt)} by {p.createdBy}
                    {p.credentialIds.length ? ` · uses ${p.credentialIds.length} credential(s)` : ''}
                  </p>
                </div>
                <div className="text-right">
                  <p className={`text-sm font-medium ${p.upToDate === p.targeted ? 'text-success' : 'text-warning'}`}>
                    {p.upToDate}/{p.targeted} up to date
                  </p>
                  {p.pending.length > 0 && <p className="text-xs text-foreground-tertiary">Pending: {p.pending.slice(0, 5).map((m) => m.name).join(', ')}{p.pending.length > 5 ? '…' : ''}</p>}
                  {canWrite && (
                    <div className="mt-2 flex justify-end gap-2">
                      <Button size="sm" variant="outline" onClick={() => setDraft({ id: p.id, name: p.name, description: p.description, all: p.target.all, tags: p.target.tags.join(', '), enabled: p.enabled, inherit: p.inherit, locked: p.locked, job: JSON.stringify(p.job, null, 2) })}>
                        Edit
                      </Button>
                      <Button size="sm" variant="ghost" onClick={() => void remove(p)}>
                        Delete
                      </Button>
                    </div>
                  )}
                </div>
              </div>
            </Card>
          ))}
        </div>
      )}

      {!!data?.inherited.length && (
        <div className="mt-6 space-y-3">
          <h2 className="text-sm font-semibold text-foreground">From parent organizations</h2>
          {data.inherited.map((i) => (
            <Card key={i.policy.id} className={i.optedOut && !i.policy.locked ? 'opacity-60' : ''}>
              <div className="flex flex-col gap-3 md:flex-row md:items-start md:justify-between">
                <div>
                  <h3 className="font-semibold text-foreground">
                    {i.policy.name} <span className="text-xs font-normal text-foreground-tertiary">v{i.policy.version} · from {i.sourceOrgName}</span>
                    {i.policy.locked && <span className="ml-2 text-xs text-info">locked</span>}
                    {i.optedOut && !i.policy.locked && <span className="ml-2 text-xs text-warning">{i.optedOutHere ? 'opted out' : 'opted out by a parent organization'}</span>}
                  </h3>
                  <div className="mt-2 flex flex-wrap items-center gap-1 text-xs text-foreground-secondary">
                    Targets: {i.policy.target.all ? 'every machine' : i.policy.target.tags.length ? i.policy.target.tags.map((t) => <TagChip key={t} tag={t} />) : 'specific machines'}
                  </div>
                  <p className="mt-1 text-xs text-foreground-tertiary">
                    {i.policy.locked ? 'Applies here and cannot be opted out of or replaced.' : 'A policy of the same name here replaces it.'}
                  </p>
                </div>
                <div className="text-right">
                  <p className="text-sm text-foreground-secondary">
                    {i.upToDate}/{i.targeted} up to date
                  </p>
                  {canWrite && !i.policy.locked && (i.optedOutHere || !i.optedOut) && (
                    <Button className="mt-2" size="sm" variant="outline" onClick={() => void optOut(i, !i.optedOutHere)}>
                      {i.optedOutHere ? 'Opt back in' : 'Opt out'}
                    </Button>
                  )}
                </div>
              </div>
            </Card>
          ))}
        </div>
      )}

      <Dialog open={!!draft} onClose={() => setDraft(null)} className="max-w-3xl">
        <DialogHeader>
          <DialogTitle>{draft?.id ? 'Edit policy' : 'New fleet policy'}</DialogTitle>
          <DialogDescription>
            The job template uses the desktop app&apos;s job format. Refer to secrets as <code>central:&lt;credential id&gt;</code>. Machines receive them sealed and never store them in plain text.
          </DialogDescription>
        </DialogHeader>
        {draft && (
          <div className="max-h-[60vh] space-y-3 overflow-y-auto pr-1">
            <ErrorBanner message={formErr} />
            <div className="grid grid-cols-1 gap-3 md:grid-cols-2">
              <Input label="Name" value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} />
              <Input label="Description" value={draft.description} onChange={(e) => setDraft({ ...draft, description: e.target.value })} />
            </div>
            <div className="flex flex-wrap items-center gap-4 text-sm">
              <label className="flex items-center gap-2">
                <input type="checkbox" checked={draft.all} onChange={(e) => setDraft({ ...draft, all: e.target.checked })} />
                All machines in this organization
              </label>
              <label className="flex items-center gap-2">
                <input type="checkbox" checked={draft.enabled} onChange={(e) => setDraft({ ...draft, enabled: e.target.checked })} />
                Enabled
              </label>
              <label className="flex items-center gap-2">
                <input type="checkbox" checked={draft.inherit} onChange={(e) => setDraft({ ...draft, inherit: e.target.checked })} />
                Also apply to child organizations
              </label>
              <label className="flex items-center gap-2">
                <input type="checkbox" disabled={!draft.inherit} checked={draft.inherit && draft.locked} onChange={(e) => setDraft({ ...draft, locked: e.target.checked })} />
                Lock (children cannot opt out or replace it)
              </label>
            </div>
            {!draft.all && <Input label="Machines with all of these tags" placeholder="laptop, finance" value={draft.tags} onChange={(e) => setDraft({ ...draft, tags: e.target.value })} />}
            <label className="flex flex-col gap-1.5 text-sm font-medium text-foreground">
              Job template (JSON)
              <textarea className="min-h-64 rounded-md border border-border bg-background p-2 font-mono text-xs" value={draft.job} onChange={(e) => setDraft({ ...draft, job: e.target.value })} spellCheck={false} />
            </label>
            {!!creds.data?.credentials.length && (
              <div className="text-xs text-foreground-secondary">
                Credentials:{' '}
                {creds.data.credentials.map((c) => (
                  <code key={c.id} className="mr-2">
                    {c.name} = {c.id}
                  </code>
                ))}
              </div>
            )}
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
