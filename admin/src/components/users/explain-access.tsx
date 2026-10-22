'use client';

/** Users → Explain: where a member's role comes from, what it allows, and the rules that gate sign-in. */
import React, { useState } from 'react';
import { Dialog, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Select } from '@/components/ui/select';
import { Badge } from '@/components/ui/badge';
import { ErrorBanner } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { ROLE_LABELS, ROLE_PERMISSIONS, type AdminUser } from '@/lib/types/auth';
import type { AccessExplanation } from '@/lib/explain';

const ALL_PERMISSIONS = [...new Set(Object.values(ROLE_PERMISSIONS).flat())].sort();

function Body({ user }: { user: AdminUser }) {
  const [permission, setPermission] = useState('');
  const { data, error } = useApi<AccessExplanation>(`/api/users/${user.id}/access${permission ? `?permission=${encodeURIComponent(permission)}` : ''}`);
  if (error) return <ErrorBanner message={error} />;
  if (!data) return <p className="text-sm text-foreground-secondary">Loading…</p>;
  return (
    <div className="space-y-4 text-sm">
      <p>
        Effective role in {data.org.name}: <strong>{data.role ? ROLE_LABELS[data.role] : 'none (no access)'}</strong>
        {data.scoped && <span className="text-foreground-tertiary"> — limited to the machines its groups grant</span>}
      </p>
      <section>
        <h4 className="mb-1 font-semibold">Where it comes from</h4>
        {data.sources.length === 0 && <p className="text-foreground-tertiary">No membership or group grants.</p>}
        <ul className="space-y-1">
          {data.sources.map((s, i) => (
            <li key={i} className={s.applies ? '' : 'text-foreground-tertiary line-through decoration-1'}>
              <Badge variant={s.applies ? 'success' : 'default'}>{ROLE_LABELS[s.role]}</Badge> {s.via === 'membership' ? 'Direct membership' : `Group “${s.groupName}”`} in{' '}
              {s.inherited ? `parent “${s.orgName}”` : `“${s.orgName}”`}
              {s.note && <span className="block pl-2 text-xs no-underline">{s.note}</span>}
            </li>
          ))}
        </ul>
        <p className="mt-1 text-xs text-foreground-tertiary">The highest applying role wins.</p>
      </section>
      {data.grants.length > 0 && (
        <section>
          <h4 className="mb-1 font-semibold">Machine access from groups{data.scoped ? '' : ' (not limiting: the role sees every machine)'}</h4>
          <ul className="text-xs">
            {data.grants.map((g, i) => (
              <li key={i}>
                {g.resourceType} {g.resourceId} — via “{g.groupName}”
              </li>
            ))}
          </ul>
        </section>
      )}
      <section>
        <h4 className="mb-1 font-semibold">Sign-in rules</h4>
        <ul className="space-y-0.5">
          {data.conditions.map((c) => (
            <li key={c.key} className={c.ok ? '' : 'text-error'}>
              {c.ok ? '✓' : '✗'} {c.detail}
            </li>
          ))}
        </ul>
      </section>
      <section>
        <Select label="Can they…" options={[{ value: '', label: 'Pick a permission' }, ...ALL_PERMISSIONS.map((p) => ({ value: p, label: p }))]} value={permission} onChange={(e) => setPermission(e.target.value)} />
        {data.check && (
          <p className={`mt-1 ${data.check.allowed ? 'text-success' : 'text-error'}`}>
            {data.check.allowed ? 'Yes' : 'No'}: {data.check.reason}
          </p>
        )}
        <details className="mt-2">
          <summary className="cursor-pointer text-xs">All {data.permissions.length} permissions</summary>
          <div className="mt-1 flex flex-wrap gap-1">
            {data.permissions.map((p) => (
              <Badge key={p} variant="default" className="text-xs">
                {p}
              </Badge>
            ))}
          </div>
        </details>
      </section>
    </div>
  );
}

export function ExplainAccessDialog({ user, onClose }: { user: AdminUser | null; onClose: () => void }) {
  if (!user) return null;
  return (
    <Dialog open onClose={onClose} className="max-w-xl">
      <DialogHeader>
        <DialogTitle>Access for {user.name}</DialogTitle>
        <DialogDescription>{user.email}</DialogDescription>
      </DialogHeader>
      <Body user={user} />
      <DialogFooter>
        <Button variant="secondary" onClick={onClose}>
          Close
        </Button>
      </DialogFooter>
    </Dialog>
  );
}
