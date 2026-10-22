'use client';

import React, { useState } from 'react';
import Link from 'next/link';
import { HardDrive, Palette } from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Select } from '@/components/ui/select';
import { SearchInput } from '@/components/ui/search-input';
import { EmptyState } from '@/components/ui/empty-state';
import { Dialog, DialogFooter, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table';
import { ConnectivityBadge, ErrorBanner, Loading, StatusBadge, TagChip } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { useAuthStore } from '@/lib/stores/auth-store';
import { formatBytes, formatRelativeTime } from '@/lib/utils/format';
import type { Machine } from '@/lib/db/repos/machines';

type Row = Machine & { backups: number; failing: number; backendSize: number; openAlerts: number; policyCompliance: { expected: number; applied: number } };
type TagInfo = { tag: string; count: number; color: string | null };

const PALETTE = ['#2563eb', '#16a34a', '#d97706', '#dc2626', '#7c3aed', '#0891b2', '#db2777', '#4b5563'];

export default function MachinesPage() {
  const [search, setSearch] = useState('');
  const [connectivity, setConnectivity] = useState('');
  const [status, setStatus] = useState('');
  const [tags, setTags] = useState<string[]>([]);
  const [colorsOpen, setColorsOpen] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const canWrite = useAuthStore((s) => s.checkPermission('fleet.write'));
  const q = new URLSearchParams();
  if (search) q.set('search', search);
  if (connectivity) q.set('connectivity', connectivity);
  if (status) q.set('status', status);
  tags.forEach((t) => q.append('tag', t));
  const { data, error, loading } = useApi<{ machines: Row[] }>(`/api/fleet/machines?${q}`, 30_000);
  const tagInfo = useApi<{ tags: TagInfo[] }>('/api/fleet/tags');
  const colorOf = (t: string) => tagInfo.data?.tags.find((x) => x.tag === t)?.color ?? null;
  const toggleTag = (t: string) => setTags((cur) => (cur.includes(t) ? cur.filter((x) => x !== t) : [...cur, t]));

  const setColor = async (tag: string, color: string | null) => {
    try {
      await api('/api/fleet/tags', { method: 'PUT', body: { tag, color } });
      await tagInfo.reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };

  return (
    <div>
      <PageHeader
        title="Machines"
        description="Every computer and server that reports to this organization."
        actions={
          <div className="flex gap-2">
            {canWrite && (
              <Button variant="outline" onClick={() => setColorsOpen(true)}>
                <Palette size={14} className="mr-2" aria-hidden="true" />
                Tag colors
              </Button>
            )}
            <Link href="/fleet/enroll">
              <Button>Enroll machines</Button>
            </Link>
          </div>
        }
      />
      <ErrorBanner message={error ?? err} />
      <Card padding="sm" className="mb-4">
        <div className="flex flex-col gap-3 md:flex-row md:items-end">
          <div className="flex-1">
            <SearchInput placeholder="Search name, host or notes" value={search} onChange={(e) => setSearch(e.target.value)} />
          </div>
          <div className="md:w-48">
            <Select
              aria-label="Connectivity"
              value={connectivity}
              onChange={(e) => setConnectivity(e.target.value)}
              placeholder="Any connectivity"
              options={[
                { value: 'online', label: 'Online' },
                { value: 'stale', label: 'Not seen recently' },
                { value: 'offline', label: 'Offline' },
                { value: 'never', label: 'Never connected' },
              ]}
            />
          </div>
          <div className="md:w-40">
            <Select
              aria-label="Status"
              value={status}
              onChange={(e) => setStatus(e.target.value)}
              placeholder="Active & suspended"
              options={[
                { value: 'active', label: 'Active' },
                { value: 'suspended', label: 'Suspended' },
                { value: 'revoked', label: 'Revoked' },
              ]}
            />
          </div>
        </div>
        {!!tagInfo.data?.tags.length && (
          <div className="mt-3 flex flex-wrap items-center gap-1.5">
            <span className="text-xs text-foreground-secondary">Tags (all must match):</span>
            {tagInfo.data.tags.map((t) => (
              <TagChip key={t.tag} tag={t.tag} color={t.color} active={tags.includes(t.tag)} onClick={() => toggleTag(t.tag)} />
            ))}
          </div>
        )}
      </Card>
      {loading && !data ? (
        <Loading />
      ) : !data?.machines.length ? (
        <Card>
          <EmptyState icon={HardDrive} title="No machines" description={q.toString() ? 'No machine matches these filters.' : 'Enroll a machine with a registration link or a claim code.'} />
        </Card>
      ) : (
        <Card padding="none">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Machine</TableHead>
                <TableHead>Connectivity</TableHead>
                <TableHead>Backups</TableHead>
                <TableHead className="hidden lg:table-cell">Stored</TableHead>
                <TableHead className="hidden lg:table-cell">Policies</TableHead>
                <TableHead className="hidden md:table-cell">Tags</TableHead>
                <TableHead className="hidden md:table-cell">Last seen</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {data.machines.map((m) => (
                <TableRow key={m.id}>
                  <TableCell>
                    <Link href={`/fleet/machines/${m.id}`} className="font-medium text-primary">
                      {m.name}
                    </Link>
                    <p className="text-xs text-foreground-tertiary">
                      {m.os} {m.osVersion} · {m.agentKind} {m.clientVersion}
                    </p>
                    {m.status !== 'active' && <StatusBadge value={m.status} />}
                  </TableCell>
                  <TableCell>
                    <ConnectivityBadge value={m.connectivity} />
                  </TableCell>
                  <TableCell>
                    {m.backups}
                    {m.failing > 0 && <span className="ml-2 text-xs text-error">{m.failing} failing</span>}
                    {m.openAlerts > 0 && <span className="ml-2 text-xs text-warning">{m.openAlerts} alert(s)</span>}
                  </TableCell>
                  <TableCell className="hidden lg:table-cell">{formatBytes(m.backendSize)}</TableCell>
                  <TableCell className="hidden lg:table-cell">
                    {m.policyCompliance.expected ? `${m.policyCompliance.applied}/${m.policyCompliance.expected} applied` : '—'}
                  </TableCell>
                  <TableCell className="hidden md:table-cell">
                    <div className="flex flex-wrap gap-1">
                      {m.tags.map((t) => (
                        <TagChip key={t} tag={t} color={colorOf(t)} />
                      ))}
                    </div>
                  </TableCell>
                  <TableCell className="hidden md:table-cell text-foreground-secondary">{m.lastSeenAt ? formatRelativeTime(m.lastSeenAt) : 'never'}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </Card>
      )}

      <Dialog open={colorsOpen} onClose={() => setColorsOpen(false)}>
        <DialogHeader>
          <DialogTitle>Tag colors</DialogTitle>
          <DialogDescription>Colors are shared by everyone in this organization.</DialogDescription>
        </DialogHeader>
        <div className="max-h-80 space-y-3 overflow-y-auto">
          {tagInfo.data?.tags.map((t) => (
            <div key={t.tag} className="flex items-center justify-between gap-3">
              <TagChip tag={t.tag} color={t.color} />
              <div className="flex items-center gap-1">
                {PALETTE.map((c) => (
                  <button key={c} type="button" aria-label={`Color ${t.tag} ${c}`} className={`h-5 w-5 rounded-full border-2 ${t.color === c ? 'border-foreground' : 'border-transparent'}`} style={{ backgroundColor: c }} onClick={() => void setColor(t.tag, c)} />
                ))}
                <button type="button" className="ml-1 text-xs text-foreground-secondary underline" onClick={() => void setColor(t.tag, null)}>
                  none
                </button>
              </div>
            </div>
          ))}
          {!tagInfo.data?.tags.length && <p className="text-sm text-foreground-secondary">No tags yet. Add tags on a machine&apos;s page.</p>}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => setColorsOpen(false)}>
            Done
          </Button>
        </DialogFooter>
      </Dialog>
    </div>
  );
}
