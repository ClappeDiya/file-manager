'use client';

import React, { useState } from 'react';
import { Plus, UsersRound } from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { EmptyState } from '@/components/ui/empty-state';
import { Dialog, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { ErrorBanner, Loading, TagChip } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { useAuthStore } from '@/lib/stores/auth-store';
import { useToastStore } from '@/lib/stores/toast-store';
import { ROLE_LABELS, type Role } from '@/lib/types/auth';
import type { Grant, Group, GroupRole } from '@/lib/db/repos/groups';

type Member = { userId: string; role: GroupRole; email: string; name: string };
type Row = Group & { members: Member[]; grants: Grant[]; canManage: boolean };

const GRANTABLE_ROLES: Role[] = ['user', 'read_only_auditor', 'operator', 'org_admin'];

export default function GroupsPage() {
  const { data, error, loading, reload } = useApi<{ groups: Row[] }>('/api/groups');
  const check = useAuthStore((s) => s.checkPermission);
  const me = useAuthStore((s) => s.user);
  const notify = useToastStore((s) => s.notify);
  const isAdmin = check('groups.write');
  const machines = useApi<{ machines: { id: string; name: string }[] }>(isAdmin ? '/api/fleet/machines' : null);
  const tags = useApi<{ tags: { tag: string }[] }>(isAdmin ? '/api/fleet/tags' : null);
  const [err, setErr] = useState<string | null>(null);
  const [draft, setDraft] = useState<{ id: string | null; name: string; description: string; orgRole: string; delegate: boolean } | null>(null);
  const [invite, setInvite] = useState<Record<string, { email: string; role: GroupRole }>>({});
  const [grantDraft, setGrantDraft] = useState<Record<string, { type: Grant['resourceType']; id: string }>>({});
  const [inviteLink, setInviteLink] = useState<string | null>(null);

  const act = async (fn: () => Promise<unknown>) => {
    setErr(null);
    try {
      await fn();
      await reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };

  const saveGroup = () =>
    act(async () => {
      if (!draft) return;
      const body = { name: draft.name, description: draft.description, orgRole: draft.orgRole || null, delegateToChildren: draft.delegate };
      await api(draft.id ? `/api/groups/${draft.id}` : '/api/groups', { method: draft.id ? 'PATCH' : 'POST', body });
      setDraft(null);
    });

  const addMember = (g: Row) =>
    act(async () => {
      const v = invite[g.id];
      if (!v?.email) return;
      const r = await api<{ added?: string; link?: string; emailed?: boolean }>(`/api/groups/${g.id}/members`, { body: { email: v.email.trim(), role: v.role } });
      setInvite({ ...invite, [g.id]: { email: '', role: 'member' } });
      if (r.link) {
        if (r.emailed) notify(`Invitation emailed to ${v.email}`, 'success');
        else setInviteLink(r.link);
      }
    });

  const addGrant = (g: Row) =>
    act(async () => {
      const v = grantDraft[g.id];
      if (!v?.id) return;
      await api(`/api/groups/${g.id}/grants`, { method: 'PUT', body: { grants: [...g.grants, { resourceType: v.type, resourceId: v.id }] } });
      setGrantDraft({ ...grantDraft, [g.id]: { type: v.type, id: '' } });
    });

  const machineName = (id: string) => machines.data?.machines.find((m) => m.id === id)?.name ?? id;

  return (
    <div>
      <PageHeader
        title="Groups"
        description="Groups give people an organization role and access to specific machines, tags or backups. Group owners manage their members."
        actions={
          isAdmin && (
            <Button onClick={() => setDraft({ id: null, name: '', description: '', orgRole: '', delegate: false })}>
              <Plus size={14} className="mr-2" aria-hidden="true" />
              New group
            </Button>
          )
        }
      />
      <ErrorBanner message={err ?? error} />
      {loading && !data ? (
        <Loading />
      ) : !data?.groups.length ? (
        <Card>
          <EmptyState icon={UsersRound} title="No groups" description="Create a group, then add people or invite them by email." />
        </Card>
      ) : (
        <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
          {data.groups.map((g) => (
            <Card key={g.id}>
              <div className="mb-3 flex items-start justify-between gap-2">
                <div>
                  <h3 className="font-semibold">{g.name}</h3>
                  {g.description && <p className="text-sm text-foreground-secondary">{g.description}</p>}
                  <p className="text-xs text-foreground-tertiary">
                    {g.orgRole ? `Grants ${ROLE_LABELS[g.orgRole]}` : 'No organization role'}
                    {g.delegateToChildren ? ' · also in child organizations' : ''}
                  </p>
                </div>
                {isAdmin && (
                  <div className="flex gap-1">
                    <Button size="sm" variant="ghost" onClick={() => setDraft({ id: g.id, name: g.name, description: g.description, orgRole: g.orgRole ?? '', delegate: g.delegateToChildren })}>
                      Edit
                    </Button>
                    <Button size="sm" variant="ghost" onClick={() => confirm(`Delete group "${g.name}"?`) && void act(() => api(`/api/groups/${g.id}`, { method: 'DELETE' }))}>
                      Delete
                    </Button>
                  </div>
                )}
              </div>

              <h4 className="mb-1 text-xs font-semibold uppercase text-foreground-secondary">Members ({g.members.length})</h4>
              <ul className="mb-3 divide-y divide-border">
                {g.members.map((m) => (
                  <li key={m.userId} className="flex items-center justify-between py-1.5 text-sm">
                    <span>
                      {m.name || m.email} <span className="text-xs text-foreground-tertiary">{m.email}</span>
                    </span>
                    <span className="flex items-center gap-1">
                      {g.canManage ? (
                        <select aria-label={`Role of ${m.email}`} className="h-7 rounded border border-border bg-background px-1 text-xs" value={m.role} onChange={(e) => void act(() => api(`/api/groups/${g.id}/members`, { method: 'PATCH', body: { userId: m.userId, role: e.target.value } }))}>
                          <option value="member">member</option>
                          <option value="owner">owner</option>
                        </select>
                      ) : (
                        <span className="text-xs">{m.role}</span>
                      )}
                      {(g.canManage || m.userId === me?.id) && (
                        <Button size="sm" variant="ghost" onClick={() => void act(() => api(`/api/groups/${g.id}/members?userId=${m.userId}`, { method: 'DELETE' }))}>
                          {m.userId === me?.id ? 'Leave' : 'Remove'}
                        </Button>
                      )}
                    </span>
                  </li>
                ))}
              </ul>
              {g.canManage && (
                <form
                  className="mb-4 flex flex-wrap items-end gap-2"
                  onSubmit={(e) => {
                    e.preventDefault();
                    void addMember(g);
                  }}
                >
                  <div className="min-w-48 flex-1">
                    <Input aria-label="Email" placeholder="name@company.com" type="email" value={invite[g.id]?.email ?? ''} onChange={(e) => setInvite({ ...invite, [g.id]: { email: e.target.value, role: invite[g.id]?.role ?? 'member' } })} />
                  </div>
                  <select aria-label="Group role" className="h-9 rounded-md border border-border bg-background px-2 text-sm" value={invite[g.id]?.role ?? 'member'} onChange={(e) => setInvite({ ...invite, [g.id]: { email: invite[g.id]?.email ?? '', role: e.target.value as GroupRole } })}>
                    <option value="member">member</option>
                    <option value="owner">owner</option>
                  </select>
                  <Button type="submit" variant="outline">
                    Add or invite
                  </Button>
                </form>
              )}

              <h4 className="mb-1 text-xs font-semibold uppercase text-foreground-secondary">Access</h4>
              <div className="flex flex-wrap gap-1">
                {g.grants.length === 0 && <span className="text-xs text-foreground-tertiary">No resource grants (role-based access only)</span>}
                {g.grants.map((gr) => (
                  <TagChip
                    key={`${gr.resourceType}:${gr.resourceId}`}
                    tag={`${gr.resourceType}: ${gr.resourceType === 'machine' ? machineName(gr.resourceId) : gr.resourceId}`}
                    onRemove={isAdmin ? () => void act(() => api(`/api/groups/${g.id}/grants`, { method: 'PUT', body: { grants: g.grants.filter((x) => x !== gr) } })) : undefined}
                  />
                ))}
              </div>
              {isAdmin && (
                <div className="mt-2 flex flex-wrap items-end gap-2">
                  <select aria-label="Grant type" className="h-9 rounded-md border border-border bg-background px-2 text-sm" value={grantDraft[g.id]?.type ?? 'tag'} onChange={(e) => setGrantDraft({ ...grantDraft, [g.id]: { type: e.target.value as Grant['resourceType'], id: '' } })}>
                    <option value="tag">Tag</option>
                    <option value="machine">Machine</option>
                    <option value="backup">Backup id</option>
                  </select>
                  {(grantDraft[g.id]?.type ?? 'tag') === 'backup' ? (
                    <Input aria-label="Backup id" value={grantDraft[g.id]?.id ?? ''} onChange={(e) => setGrantDraft({ ...grantDraft, [g.id]: { type: 'backup', id: e.target.value } })} />
                  ) : (
                    <select
                      aria-label="Resource"
                      className="h-9 min-w-40 rounded-md border border-border bg-background px-2 text-sm"
                      value={grantDraft[g.id]?.id ?? ''}
                      onChange={(e) => setGrantDraft({ ...grantDraft, [g.id]: { type: grantDraft[g.id]?.type ?? 'tag', id: e.target.value } })}
                    >
                      <option value="">Choose…</option>
                      {(grantDraft[g.id]?.type ?? 'tag') === 'tag'
                        ? tags.data?.tags.map((t) => (
                            <option key={t.tag} value={t.tag}>
                              {t.tag}
                            </option>
                          ))
                        : machines.data?.machines.map((m) => (
                            <option key={m.id} value={m.id}>
                              {m.name}
                            </option>
                          ))}
                    </select>
                  )}
                  <Button variant="outline" onClick={() => void addGrant(g)}>
                    Grant
                  </Button>
                </div>
              )}
            </Card>
          ))}
        </div>
      )}

      <Dialog open={!!draft} onClose={() => setDraft(null)}>
        <DialogHeader>
          <DialogTitle>{draft?.id ? 'Edit group' : 'New group'}</DialogTitle>
          <DialogDescription>You become an owner of a group you create.</DialogDescription>
        </DialogHeader>
        {draft && (
          <div className="space-y-3">
            <Input label="Name" value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} />
            <Input label="Description" value={draft.description} onChange={(e) => setDraft({ ...draft, description: e.target.value })} />
            <Select label="Organization role for members" value={draft.orgRole} onChange={(e) => setDraft({ ...draft, orgRole: e.target.value })} placeholder="None" options={GRANTABLE_ROLES.map((r) => ({ value: r, label: ROLE_LABELS[r] }))} />
            <label className="flex items-center gap-2 text-sm">
              <input type="checkbox" checked={draft.delegate} onChange={(e) => setDraft({ ...draft, delegate: e.target.checked })} />
              Members get this role in child organizations too
            </label>
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => setDraft(null)}>
            Cancel
          </Button>
          <Button onClick={() => void saveGroup()}>Save</Button>
        </DialogFooter>
      </Dialog>

      <Dialog open={!!inviteLink} onClose={() => setInviteLink(null)}>
        <DialogHeader>
          <DialogTitle>Invitation created</DialogTitle>
          <DialogDescription>No email channel is set up for invitations. Send this link yourself. It works once.</DialogDescription>
        </DialogHeader>
        <code className="block break-all rounded bg-background-secondary p-2 text-xs">{inviteLink}</code>
        <DialogFooter>
          <Button variant="outline" onClick={() => inviteLink && void navigator.clipboard.writeText(inviteLink)}>
            Copy
          </Button>
          <Button onClick={() => setInviteLink(null)}>Done</Button>
        </DialogFooter>
      </Dialog>
    </div>
  );
}
