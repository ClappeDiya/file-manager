'use client';

import React, { useMemo, useState } from 'react';
import { Building2, Lock, Plus, Unlink } from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Dialog, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { ErrorBanner, Loading, StatusBadge } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { useAuthStore } from '@/lib/stores/auth-store';
import { ROLE_LABELS, type Role } from '@/lib/types/auth';
import { formatDate } from '@/lib/utils/format';
import type { License, Organization } from '@/lib/db/repos/orgs';

type Org = Organization & { role: Role; path: string; license: License };

interface Draft {
  mode: 'create' | 'edit' | 'offboard';
  org: Org | null;
  name: string;
  parentId: string;
  detached: boolean;
  licensePlan: string;
  licenseSeats: string;
  confirm: string;
  graceDays: string;
}

const manages = (r: Role) => r === 'org_owner' || r === 'org_admin';

export default function OrganizationsPage() {
  const { data, error, loading, reload } = useApi<{ organizations: Org[]; currentOrgId: string }>('/api/orgs');
  const { switchOrg, validateSession } = useAuthStore();
  const [draft, setDraft] = useState<Draft | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [formErr, setFormErr] = useState<string | null>(null);

  // Render as a tree: roots are organizations whose parent the user cannot see.
  const tree = useMemo(() => {
    const orgs = data?.organizations ?? [];
    const ids = new Set(orgs.map((o) => o.id));
    const kids = new Map<string | null, Org[]>();
    for (const o of orgs) {
      const p = o.parentId && ids.has(o.parentId) ? o.parentId : null;
      kids.set(p, [...(kids.get(p) ?? []), o]);
    }
    const out: { org: Org; depth: number }[] = [];
    const walk = (p: string | null, depth: number) => {
      for (const o of (kids.get(p) ?? []).sort((a, b) => a.name.localeCompare(b.name))) {
        out.push({ org: o, depth });
        walk(o.id, depth + 1);
      }
    };
    walk(null, 0);
    return out;
  }, [data]);

  const open = (mode: Draft['mode'], org: Org | null) => {
    setFormErr(null);
    setDraft({ mode, org, name: org && mode === 'edit' ? org.name : '', parentId: org?.id ?? data?.currentOrgId ?? '', detached: false, licensePlan: org?.licensePlan ?? '', licenseSeats: org?.licenseSeats?.toString() ?? '', confirm: '', graceDays: '30' });
  };

  const submit = async () => {
    if (!draft) return;
    setFormErr(null);
    try {
      if (draft.mode === 'create') {
        await api('/api/orgs', { body: { name: draft.name, parentId: draft.parentId, detached: draft.detached, licensePlan: draft.licensePlan || null, licenseSeats: draft.licenseSeats ? Number(draft.licenseSeats) : null } });
      } else if (draft.mode === 'edit' && draft.org) {
        const body: Record<string, unknown> = { name: draft.name };
        const parent = data?.organizations.find((o) => o.id === draft.org!.parentId);
        if (parent && manages(parent.role)) Object.assign(body, { licensePlan: draft.licensePlan || null, licenseSeats: draft.licenseSeats ? Number(draft.licenseSeats) : null });
        await api(`/api/orgs/${draft.org.id}`, { method: 'PATCH', body });
      } else if (draft.mode === 'offboard' && draft.org) {
        await api(`/api/orgs/${draft.org.id}`, { method: 'DELETE', body: { confirmName: draft.confirm, graceDays: Number(draft.graceDays) } });
      }
      setDraft(null);
      await reload();
      await validateSession();
    } catch (e) {
      setFormErr(errorText(e));
    }
  };

  const patch = async (o: Org, body: Record<string, unknown>) => {
    setErr(null);
    try {
      await api(`/api/orgs/${o.id}`, { method: 'PATCH', body });
      await reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };

  return (
    <div>
      <PageHeader
        title="Organizations"
        description="Organizations nest: managers of a parent also manage its children, unless a child is detached."
        actions={
          <Button onClick={() => open('create', null)}>
            <Plus size={14} className="mr-2" aria-hidden="true" />
            New organization
          </Button>
        }
      />
      <ErrorBanner message={err ?? error} />
      {loading && !data ? (
        <Loading />
      ) : (
        <Card padding="none">
          <ul className="divide-y divide-border">
            {tree.map(({ org: o, depth }) => (
              <li key={o.id} className="flex flex-col gap-2 px-4 py-3 md:flex-row md:items-center md:justify-between" style={{ paddingLeft: `${16 + depth * 24}px` }}>
                <div>
                  <p className="flex flex-wrap items-center gap-2 font-medium text-foreground">
                    <Building2 size={14} aria-hidden="true" />
                    {o.name}
                    {o.id === data?.currentOrgId && <span className="text-xs text-primary">current</span>}
                    {o.protected && <Lock size={12} aria-label="Protected" />}
                    {o.detached && <span className="inline-flex items-center gap-1 text-xs text-foreground-secondary"><Unlink size={12} aria-hidden="true" />detached</span>}
                    {o.status !== 'active' && <StatusBadge value={o.status} />}
                  </p>
                  <p className="text-xs text-foreground-tertiary">
                    {ROLE_LABELS[o.role]} · {o.license.plan ?? 'no plan'} · {o.license.used}
                    {o.license.seats !== null ? `/${o.license.seats}` : ''} machines{o.license.inherited ? ' (inherited license)' : ''}
                    {o.offboardAfter ? ` · data deleted after ${formatDate(o.offboardAfter)}` : ''}
                  </p>
                </div>
                <div className="flex flex-wrap gap-1">
                  {o.id !== data?.currentOrgId && o.status === 'active' && (
                    <Button size="sm" variant="ghost" onClick={() => void switchOrg(o.id).then(reload).catch((e) => setErr(errorText(e)))}>
                      Switch to
                    </Button>
                  )}
                  {manages(o.role) && o.status === 'active' && (
                    <>
                      <Button size="sm" variant="ghost" onClick={() => open('edit', o)}>
                        Edit
                      </Button>
                      <Button size="sm" variant="ghost" onClick={() => open('create', o)}>
                        Add child
                      </Button>
                    </>
                  )}
                  {o.role === 'org_owner' && o.status === 'active' && (
                    <>
                      <Button size="sm" variant="ghost" onClick={() => void patch(o, { protected: !o.protected })}>
                        {o.protected ? 'Unprotect' : 'Protect'}
                      </Button>
                      {o.parentId && (
                        <Button size="sm" variant="ghost" onClick={() => void patch(o, { detached: !o.detached })}>
                          {o.detached ? 'Reattach' : 'Detach'}
                        </Button>
                      )}
                      {o.id !== 'org_default' && (
                        <Button size="sm" variant="ghost" className="text-error" onClick={() => open('offboard', o)}>
                          Offboard
                        </Button>
                      )}
                    </>
                  )}
                  {o.status === 'offboarding' && manages(o.role) && (
                    <Button size="sm" variant="outline" onClick={() => void patch(o, { action: 'cancel_offboarding' })}>
                      Cancel offboarding
                    </Button>
                  )}
                </div>
              </li>
            ))}
          </ul>
        </Card>
      )}

      <Dialog open={!!draft} onClose={() => setDraft(null)}>
        <DialogHeader>
          <DialogTitle>{draft?.mode === 'create' ? 'New organization' : draft?.mode === 'edit' ? `Edit "${draft.org?.name}"` : `Offboard "${draft?.org?.name}"`}</DialogTitle>
          {draft?.mode === 'offboard' && <DialogDescription>Access is revoked now for everyone, including sub-organizations. Data is deleted for good after the grace period. A protected organization must be unprotected first.</DialogDescription>}
          {draft?.mode === 'create' && <DialogDescription>Created under {data?.organizations.find((o) => o.id === draft.parentId)?.path}.</DialogDescription>}
        </DialogHeader>
        {draft && (
          <div className="space-y-3">
            <ErrorBanner message={formErr} />
            {draft.mode !== 'offboard' ? (
              <>
                <Input label="Name" value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} />
                <div className="grid grid-cols-2 gap-3">
                  <Input label="License plan" value={draft.licensePlan} onChange={(e) => setDraft({ ...draft, licensePlan: e.target.value })} hint="Empty = inherit from parent" />
                  <Input label="Machine seats" type="number" min={0} value={draft.licenseSeats} onChange={(e) => setDraft({ ...draft, licenseSeats: e.target.value })} />
                </div>
                {draft.mode === 'create' && (
                  <label className="flex items-start gap-2 text-sm">
                    <input type="checkbox" className="mt-1" checked={draft.detached} onChange={(e) => setDraft({ ...draft, detached: e.target.checked })} />
                    <span>
                      Detached: managers of the parent organizations cannot see or manage it. You become its owner.
                    </span>
                  </label>
                )}
              </>
            ) : (
              <>
                <Input label={`Type "${draft.org?.name}" to confirm`} value={draft.confirm} onChange={(e) => setDraft({ ...draft, confirm: e.target.value })} />
                <Input label="Grace period (days)" type="number" min={0} max={90} value={draft.graceDays} onChange={(e) => setDraft({ ...draft, graceDays: e.target.value })} />
              </>
            )}
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => setDraft(null)}>
            Cancel
          </Button>
          <Button variant={draft?.mode === 'offboard' ? 'danger' : 'primary'} onClick={() => void submit()} disabled={draft?.mode === 'offboard' && draft.confirm !== draft.org?.name}>
            {draft?.mode === 'offboard' ? 'Offboard' : 'Save'}
          </Button>
        </DialogFooter>
      </Dialog>
    </div>
  );
}
