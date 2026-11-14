'use client';

import React, { useState } from 'react';
import Link from 'next/link';
import { HardDrive, CheckCircle2, AlertTriangle, XCircle, Database, Bell } from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card, CardHeader, CardTitle } from '@/components/ui/card';
import { StatCard } from '@/components/ui/stat-card';
import { Select } from '@/components/ui/select';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table';
import { StorageChart, type StorageDay } from '@/components/fleet/storage-chart';
import { ErrorBanner, Loading, StatusBadge, TagChip } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { formatBytes, formatRelativeTime } from '@/lib/utils/format';
import type { BackupRollup } from '@/lib/db/repos/reports';
import type { Alert } from '@/lib/db/repos/alerts';

interface Dashboard {
  machines: { total: number; online: number; stale: number; offline: number; never: number; suspended: number };
  backups: { total: number; success: number; warning: number; error: number; neverRun: number; sourceSize: number; backendSize: number; versions: number };
  runs: { total: number; failed: number; warnings: number; bytesUploaded: number };
  storageHistory: StorageDay[];
  largestBackups: (BackupRollup & { machineName: string })[];
  openAlerts: Alert[];
  license: { plan: string | null; seats: number | null; used: number } | null;
}

export default function FleetDashboardPage() {
  const [days, setDays] = useState('30');
  const [children, setChildren] = useState(false);
  const [tag, setTag] = useState<string | null>(null);
  const q = new URLSearchParams({ days, ...(children ? { includeChildren: '1' } : {}), ...(tag ? { tag } : {}) });
  const { data, error, loading } = useApi<Dashboard>(`/api/fleet/dashboard?${q}`, 60_000);
  const tags = useApi<{ tags: { tag: string; count: number; color: string | null }[] }>('/api/fleet/tags');

  return (
    <div>
      <PageHeader title="Fleet overview" description="Backup health, storage and alerts across every registered machine." />
      <div className="mb-4 flex flex-wrap items-end gap-3">
        <div className="w-40">
          <Select
            label="Period"
            value={days}
            onChange={(e) => setDays(e.target.value)}
            options={[
              { value: '7', label: 'Last 7 days' },
              { value: '30', label: 'Last 30 days' },
              { value: '90', label: 'Last 90 days' },
              { value: '365', label: 'Last year' },
            ]}
          />
        </div>
        <label className="flex items-center gap-2 pb-2 text-sm text-foreground">
          <input type="checkbox" checked={children} onChange={(e) => setChildren(e.target.checked)} />
          Include child organizations
        </label>
        <div className="flex flex-wrap items-center gap-1.5 pb-2">
          {tags.data?.tags.map((t) => (
            <TagChip key={t.tag} tag={`${t.tag} (${t.count})`} color={t.color} active={tag === t.tag} onClick={() => setTag(tag === t.tag ? null : t.tag)} />
          ))}
        </div>
      </div>
      <ErrorBanner message={error} />
      {loading && !data ? (
        <Loading />
      ) : data ? (
        <>
          <div className="mb-6 grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4">
            <StatCard title="Machines" value={data.machines.total} subtitle={`${data.machines.online} online · ${data.machines.stale} stale · ${data.machines.offline} offline`} icon={HardDrive} />
            <StatCard title="Healthy backups" value={`${data.backups.success}/${data.backups.total}`} subtitle={`${data.backups.neverRun} never run`} icon={CheckCircle2} variant="success" />
            <StatCard title="Failing backups" value={data.backups.error} subtitle={`${data.backups.warning} with warnings`} icon={data.backups.error ? XCircle : AlertTriangle} variant={data.backups.error ? 'error' : 'default'} />
            <StatCard title="Stored" value={formatBytes(data.backups.backendSize)} subtitle={`${formatBytes(data.backups.sourceSize)} protected · ${data.backups.versions} versions`} icon={Database} />
          </div>
          <div className="grid grid-cols-1 gap-4 xl:grid-cols-3">
            <Card className="xl:col-span-2">
              <CardHeader>
                <CardTitle>Stored data per day</CardTitle>
                <span className="text-xs text-foreground-secondary">
                  {data.runs.total} runs · {data.runs.failed} failed · {formatBytes(data.runs.bytesUploaded)} uploaded
                </span>
              </CardHeader>
              <StorageChart days={data.storageHistory} />
            </Card>
            <Card>
              <CardHeader>
                <CardTitle>Open alerts</CardTitle>
                <Link href="/alerts" className="text-sm text-primary">
                  All alerts
                </Link>
              </CardHeader>
              {data.openAlerts.length === 0 ? (
                <p className="text-sm text-foreground-secondary">Nothing needs attention.</p>
              ) : (
                <ul className="space-y-3">
                  {data.openAlerts.slice(0, 8).map((a) => (
                    <li key={a.id} className="flex gap-2 text-sm">
                      <Bell size={14} className={a.severity === 'critical' ? 'mt-0.5 text-error' : 'mt-0.5 text-warning'} aria-hidden="true" />
                      <div>
                        <p className="text-foreground">{a.message}</p>
                        <p className="text-xs text-foreground-tertiary">
                          {a.severity} · {formatRelativeTime(a.createdAt)}
                        </p>
                      </div>
                    </li>
                  ))}
                </ul>
              )}
            </Card>
          </div>
          <Card className="mt-4" padding="none">
            <div className="p-6 pb-2">
              <CardTitle>Largest backups</CardTitle>
            </div>
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Backup</TableHead>
                  <TableHead>Machine</TableHead>
                  <TableHead>Last result</TableHead>
                  <TableHead>Stored</TableHead>
                  <TableHead>Versions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {data.largestBackups.map((b) => (
                  <TableRow key={`${b.machineId}:${b.backupId}`}>
                    <TableCell className="font-medium">{b.name}</TableCell>
                    <TableCell>
                      <Link href={`/fleet/machines/${b.machineId}`} className="text-primary">
                        {b.machineName}
                      </Link>
                    </TableCell>
                    <TableCell>
                      <StatusBadge value={b.lastStatus} />
                    </TableCell>
                    <TableCell>{formatBytes(b.backendSize)}</TableCell>
                    <TableCell>{b.versionCount}</TableCell>
                  </TableRow>
                ))}
                {data.largestBackups.length === 0 && (
                  <TableRow>
                    <TableCell colSpan={5} className="text-center text-foreground-secondary">
                      No backups reported yet. <Link href="/fleet/enroll" className="text-primary">Enroll a machine</Link>.
                    </TableCell>
                  </TableRow>
                )}
              </TableBody>
            </Table>
          </Card>
          {data.license && (
            <p className="mt-4 text-xs text-foreground-tertiary">
              License: {data.license.plan ?? 'unlimited'} · {data.license.used}
              {data.license.seats ? ` of ${data.license.seats}` : ''} machines
            </p>
          )}
        </>
      ) : null}
    </div>
  );
}
