'use client';

import React, { useState } from 'react';
import { KeyRound, Plus } from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { EmptyState } from '@/components/ui/empty-state';
import { Dialog, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table';
import { ErrorBanner, Loading } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { useAuthStore } from '@/lib/stores/auth-store';
import { formatRelativeTime } from '@/lib/utils/format';
import type { CentralCredential, CredentialVersion } from '@/lib/db/repos/fleet-policies';

type CredentialRow = CentralCredential & { propagation: { targeted: number; upToDate: number; pending: { id: string; name: string; version: number | null }[] } };

/** Rotation history and the machines still on an older version. */
function CredentialDetails({ c }: { c: CredentialRow }) {
  const { data } = useApi<{ history: CredentialVersion[] }>(`/api/fleet/credentials/${c.id}`);
  return (
    <div className="grid gap-3 text-xs md:grid-cols-2">
      <div>
        <p className="mb-1 font-medium">Versions</p>
        <ul className="space-y-0.5">
          {(data?.history ?? []).map((h) => (
            <li key={h.version}>
              v{h.version} · {formatRelativeTime(h.createdAt)}
              {h.createdBy && ` by ${h.createdBy}`}
            </li>
          ))}
        </ul>
      </div>
      <div>
        <p className="mb-1 font-medium">Waiting for v{c.version}</p>
        {c.propagation.pending.length === 0 ? (
          <p className="text-foreground-tertiary">Every machine that uses it has the current version.</p>
        ) : (
          <ul className="space-y-0.5">
            {c.propagation.pending.map((m) => (
              <li key={m.id}>
                <a className="text-primary hover:underline" href={`/fleet/machines/${m.id}`}>
                  {m.name}
                </a>{' '}
                ({m.version === null ? 'not delivered yet' : `has v${m.version}`})
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

const KINDS = [
  { value: 'passphrase', label: 'Backup passphrase' },
  { value: 'access_key', label: 'Access key / secret' },
  { value: 'password', label: 'Password' },
  { value: 'token', label: 'API token' },
  { value: 'json', label: 'Service account JSON' },
];

export default function CredentialsPage() {
  const { data, error, loading, reload } = useApi<{ credentials: CredentialRow[] }>('/api/fleet/credentials', 15_000);
  const [open, setOpen] = useState<string | null>(null);
  const canWrite = useAuthStore((s) => s.checkPermission('credentials.write'));
  const [err, setErr] = useState<string | null>(null);
  const [draft, setDraft] = useState<{ id: string | null; name: string; kind: string; description: string; secret: string } | null>(null);

  const save = async () => {
    if (!draft) return;
    try {
      if (draft.id) await api(`/api/fleet/credentials/${draft.id}`, { method: 'PUT', body: { secret: draft.secret } });
      else await api('/api/fleet/credentials', { body: { name: draft.name, kind: draft.kind, description: draft.description, secret: draft.secret } });
      setDraft(null);
      await reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };

  const remove = async (c: CredentialRow) => {
    if (!confirm(`Delete "${c.name}"?`)) return;
    try {
      await api(`/api/fleet/credentials/${c.id}`, { method: 'DELETE' });
      await reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };

  return (
    <div>
      <PageHeader
        title="Credentials"
        description="Secrets stored encrypted on the console and delivered sealed to the machines whose policies use them. They can be replaced but never read back."
        actions={
          canWrite && (
            <Button onClick={() => setDraft({ id: null, name: '', kind: 'passphrase', description: '', secret: '' })}>
              <Plus size={14} className="mr-2" aria-hidden="true" />
              Add credential
            </Button>
          )
        }
      />
      <ErrorBanner message={err ?? error} />
      {loading && !data ? (
        <Loading />
      ) : !data?.credentials.length ? (
        <Card>
          <EmptyState icon={KeyRound} title="No credentials" description="Add a passphrase or cloud key, then reference it from a fleet policy as central:<id>." />
        </Card>
      ) : (
        <Card padding="none">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Name</TableHead>
                <TableHead>Kind</TableHead>
                <TableHead className="hidden md:table-cell">Reference</TableHead>
                <TableHead className="hidden md:table-cell">Updated</TableHead>
                <TableHead>Version</TableHead>
                <TableHead />
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.credentials.map((c) => (
                <React.Fragment key={c.id}>
                  <TableRow>
                  <TableCell>
                    <p className="font-medium">{c.name}</p>
                    {c.description && <p className="text-xs text-foreground-tertiary">{c.description}</p>}
                  </TableCell>
                  <TableCell>{KINDS.find((k) => k.value === c.kind)?.label ?? c.kind}</TableCell>
                  <TableCell className="hidden md:table-cell font-mono text-xs">central:{c.id}</TableCell>
                  <TableCell className="hidden md:table-cell">{formatRelativeTime(c.updatedAt)}</TableCell>
                  <TableCell>
                    <button type="button" className="text-left text-xs hover:underline" aria-expanded={open === c.id} onClick={() => setOpen(open === c.id ? null : c.id)}>
                      v{c.version}
                      {c.propagation.targeted > 0 && (
                        <span className={c.propagation.upToDate === c.propagation.targeted ? 'text-foreground-tertiary' : 'text-amber-600'}>
                          {' '}
                          · {c.propagation.upToDate}/{c.propagation.targeted} machines
                        </span>
                      )}
                    </button>
                  </TableCell>
                  <TableCell className="text-right">
                    {canWrite && (
                      <div className="flex justify-end gap-1">
                        <Button size="sm" variant="ghost" onClick={() => setDraft({ id: c.id, name: c.name, kind: c.kind, description: c.description, secret: '' })}>
                          Replace secret
                        </Button>
                        <Button size="sm" variant="ghost" onClick={() => void remove(c)}>
                          Delete
                        </Button>
                      </div>
                    )}
                  </TableCell>
                  </TableRow>
                  {open === c.id && (
                    <TableRow>
                      <TableCell colSpan={6}>
                        <CredentialDetails c={c} />
                      </TableCell>
                    </TableRow>
                  )}
                </React.Fragment>
              ))}
            </TableBody>
          </Table>
        </Card>
      )}

      <Dialog open={!!draft} onClose={() => setDraft(null)}>
        <DialogHeader>
          <DialogTitle>{draft?.id ? `Replace secret of "${draft.name}"` : 'Add credential'}</DialogTitle>
          <DialogDescription>Machines pick up a replaced secret on their next policy check-in.</DialogDescription>
        </DialogHeader>
        {draft && (
          <div className="space-y-3">
            {!draft.id && (
              <>
                <Input label="Name" value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} />
                <Select label="Kind" value={draft.kind} onChange={(e) => setDraft({ ...draft, kind: e.target.value })} options={KINDS} />
                <Input label="Description" value={draft.description} onChange={(e) => setDraft({ ...draft, description: e.target.value })} />
              </>
            )}
            {draft.kind === 'json' ? (
              <label className="flex flex-col gap-1.5 text-sm font-medium text-foreground">
                Secret
                <textarea className="min-h-32 rounded-md border border-border bg-background p-2 font-mono text-xs" value={draft.secret} onChange={(e) => setDraft({ ...draft, secret: e.target.value })} autoComplete="off" spellCheck={false} />
              </label>
            ) : (
              <Input label="Secret" type="password" autoComplete="new-password" value={draft.secret} onChange={(e) => setDraft({ ...draft, secret: e.target.value })} />
            )}
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => setDraft(null)}>
            Cancel
          </Button>
          <Button onClick={() => void save()} disabled={!draft?.secret}>
            Save
          </Button>
        </DialogFooter>
      </Dialog>
    </div>
  );
}
