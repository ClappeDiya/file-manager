'use client';

import React, { useEffect, useState } from 'react';
import { Copy, Download, Link2, Plus } from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Dialog, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table';
import { ErrorBanner, Loading, StatusBadge, TagChip } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { useToastStore } from '@/lib/stores/toast-store';
import { formatDateTime, formatRelativeTime } from '@/lib/utils/format';
import type { RegistrationLink } from '@/lib/db/repos/machines';

interface Created {
  link: RegistrationLink;
  token: string;
  consoleUrl: string;
  reportUrl: string;
}

const splitTags = (s: string) => s.split(',').map((t) => t.trim()).filter(Boolean);

function CopyField({ label, value }: { label: string; value: string }) {
  const notify = useToastStore((s) => s.notify);
  return (
    <div>
      <p className="mb-1 text-xs font-medium text-foreground-secondary">{label}</p>
      <div className="flex items-start gap-2">
        <code className="flex-1 break-all rounded bg-background-secondary p-2 text-xs">{value}</code>
        <Button size="icon" variant="ghost" aria-label={`Copy ${label}`} onClick={() => void navigator.clipboard.writeText(value).then(() => notify('Copied', 'success'))}>
          <Copy size={14} />
        </Button>
      </div>
    </div>
  );
}

export default function EnrollPage() {
  const { data, error, loading, reload } = useApi<{ links: RegistrationLink[] }>('/api/fleet/links');
  const [err, setErr] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const [label, setLabel] = useState('');
  const [hours, setHours] = useState('168');
  const [maxUses, setMaxUses] = useState('');
  const [tags, setTags] = useState('');
  const [created, setCreated] = useState<Created | null>(null);
  const [code, setCode] = useState('');
  const [claimName, setClaimName] = useState('');
  const [claimTags, setClaimTags] = useState('');
  const [claimed, setClaimed] = useState<string | null>(null);
  const [origin, setOrigin] = useState('');
  useEffect(() => setOrigin(window.location.origin), []);

  const create = async () => {
    setErr(null);
    try {
      const r = await api<Created>('/api/fleet/links', {
        body: { label, expiresInHours: hours ? Number(hours) : null, maxUses: maxUses ? Number(maxUses) : null, defaultTags: splitTags(tags) },
      });
      setCreated(r);
      setOpen(false);
      setLabel('');
      await reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };

  const revoke = async (id: string) => {
    try {
      await api(`/api/fleet/links/${id}`, { method: 'DELETE' });
      await reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };

  const claim = async (e: React.FormEvent) => {
    e.preventDefault();
    setErr(null);
    setClaimed(null);
    try {
      const r = await api<{ machine: { id: string; name: string } }>('/api/fleet/claims', { body: { code: code.trim().toUpperCase(), name: claimName || undefined, tags: splitTags(claimTags) } });
      setClaimed(r.machine.name);
      setCode('');
      setClaimName('');
    } catch (e2) {
      setErr(errorText(e2));
    }
  };

  return (
    <div>
      <PageHeader
        title="Enrollment"
        description="Connect machines with a registration link (unattended installs) or by claiming the code a machine shows."
        actions={
          <Button onClick={() => setOpen(true)}>
            <Plus size={14} className="mr-2" aria-hidden="true" />
            New registration link
          </Button>
        }
      />
      <ErrorBanner message={err ?? error} />

      <div className="mb-4 grid grid-cols-1 gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader className="flex-col items-start gap-1">
            <CardTitle>Claim a machine</CardTitle>
            <CardDescription>
              On the machine run <code>ufop agent claim --url {origin}</code> (or choose &quot;Connect to console&quot; in the app), then enter the code it shows.
            </CardDescription>
          </CardHeader>
          <form onSubmit={claim} className="space-y-3">
            <Input label="Claim code" placeholder="ABCD-EFGH" value={code} onChange={(e) => setCode(e.target.value)} required autoComplete="off" />
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <Input label="Display name (optional)" value={claimName} onChange={(e) => setClaimName(e.target.value)} />
              <Input label="Tags (comma separated)" value={claimTags} onChange={(e) => setClaimTags(e.target.value)} />
            </div>
            <Button type="submit">Claim</Button>
            {claimed && <p className="text-sm text-success">Claimed “{claimed}”. It connects within a few seconds.</p>}
          </form>
        </Card>
        <Card>
          <CardHeader className="flex-col items-start gap-1">
            <CardTitle>How registration links work</CardTitle>
          </CardHeader>
          <ul className="list-disc space-y-1 pl-5 text-sm text-foreground-secondary">
            <li>The token is shown once. Store it in your deployment tool.</li>
            <li>
              Pass it to <code>ufop agent register</code>, or download a <code>preload.json</code> for silent installs. The app, <code>ufop-server</code> and the CLI read it on first start.
            </li>
            <li>Machines that register get the link&apos;s default tags.</li>
            <li>Revoking a link stops new registrations. Machines that already registered keep working.</li>
          </ul>
        </Card>
      </div>

      {loading && !data ? (
        <Loading />
      ) : (
        <Card padding="none">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Link</TableHead>
                <TableHead>Uses</TableHead>
                <TableHead className="hidden md:table-cell">Expires</TableHead>
                <TableHead className="hidden md:table-cell">Default tags</TableHead>
                <TableHead>State</TableHead>
                <TableHead />
              </TableRow>
            </TableHeader>
            <TableBody>
              {data?.links.map((l) => (
                <TableRow key={l.id}>
                  <TableCell>
                    <p className="font-medium">{l.label || 'Untitled link'}</p>
                    <p className="font-mono text-xs text-foreground-tertiary">{l.tokenPrefix}…</p>
                    <p className="text-xs text-foreground-tertiary">
                      by {l.createdBy}, {formatRelativeTime(l.createdAt)}
                    </p>
                  </TableCell>
                  <TableCell>
                    {l.uses}
                    {l.maxUses !== null ? ` / ${l.maxUses}` : ''}
                  </TableCell>
                  <TableCell className="hidden md:table-cell">{l.expiresAt ? formatDateTime(l.expiresAt) : 'never'}</TableCell>
                  <TableCell className="hidden md:table-cell">
                    <div className="flex flex-wrap gap-1">
                      {l.defaultTags.map((t) => (
                        <TagChip key={t} tag={t} />
                      ))}
                    </div>
                  </TableCell>
                  <TableCell>{l.revokedAt ? <StatusBadge value="revoked" /> : l.usable ? <StatusBadge value="active" /> : <StatusBadge value="expired" />}</TableCell>
                  <TableCell className="text-right">
                    {l.usable && (
                      <div className="flex justify-end gap-1">
                        <a href={`/api/fleet/links/${l.id}/preload`} download>
                          <Button size="sm" variant="ghost">
                            <Download size={12} className="mr-1" aria-hidden="true" />
                            preload.json
                          </Button>
                        </a>
                        <Button size="sm" variant="ghost" onClick={() => void revoke(l.id)}>
                          Revoke
                        </Button>
                      </div>
                    )}
                  </TableCell>
                </TableRow>
              ))}
              {!data?.links.length && (
                <TableRow>
                  <TableCell colSpan={6} className="py-8 text-center text-foreground-secondary">
                    <Link2 size={16} className="mr-2 inline" aria-hidden="true" />
                    No registration links yet.
                  </TableCell>
                </TableRow>
              )}
            </TableBody>
          </Table>
        </Card>
      )}

      <Dialog open={open} onClose={() => setOpen(false)}>
        <DialogHeader>
          <DialogTitle>New registration link</DialogTitle>
          <DialogDescription>Anyone with the token can add a machine to this organization until it expires or runs out of uses.</DialogDescription>
        </DialogHeader>
        <div className="space-y-3">
          <Input label="Label" placeholder="Laptops rollout" value={label} onChange={(e) => setLabel(e.target.value)} />
          <div className="grid grid-cols-2 gap-3">
            <Input label="Expires after (hours)" type="number" min={1} value={hours} onChange={(e) => setHours(e.target.value)} hint="Empty = never" />
            <Input label="Maximum uses" type="number" min={1} value={maxUses} onChange={(e) => setMaxUses(e.target.value)} hint="Empty = unlimited" />
          </div>
          <Input label="Default tags" placeholder="laptop, finance" value={tags} onChange={(e) => setTags(e.target.value)} />
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => setOpen(false)}>
            Cancel
          </Button>
          <Button onClick={() => void create()}>Create link</Button>
        </DialogFooter>
      </Dialog>

      <Dialog open={!!created} onClose={() => setCreated(null)} className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>Registration link created</DialogTitle>
          <DialogDescription>Copy the token now. It is not shown again.</DialogDescription>
        </DialogHeader>
        {created && (
          <div className="space-y-3">
            <CopyField label="Token" value={created.token} />
            <CopyField label="Register from the command line" value={`ufop agent register --url ${created.consoleUrl} --token ${created.token}${created.link.defaultTags.length ? ` --tags ${created.link.defaultTags.join(',')}` : ''}`} />
            <CopyField label="Report URL (for clients that only send reports)" value={created.reportUrl} />
          </div>
        )}
        <DialogFooter>
          {created && (
            <a href={`/api/fleet/links/${created.link.id}/preload`} download>
              <Button variant="outline">
                <Download size={14} className="mr-2" aria-hidden="true" />
                preload.json
              </Button>
            </a>
          )}
          <Button onClick={() => setCreated(null)}>Done</Button>
        </DialogFooter>
      </Dialog>
    </div>
  );
}
