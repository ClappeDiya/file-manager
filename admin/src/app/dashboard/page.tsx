'use client';

import { useRouter } from 'next/navigation';
import React, { useState, useEffect, useCallback } from 'react';
import {
  Users, Monitor, Shield, CheckSquare, ScrollText,
  AlertTriangle, Activity, ArrowUpRight, Clock, RefreshCw,
} from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { StatCard } from '@/components/ui/stat-card';
import { Card, CardHeader, CardTitle, CardContent } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { formatRelativeTime, formatBytes } from '@/lib/utils/format';
import type { AdminUser } from '@/lib/types/auth';
import type { PolicyRule } from '@/lib/types/policies';
import type { ApprovalRequest } from '@/lib/types/approvals';
import type { AuditEntry, DeviceHealth } from '@/lib/types/audit';
import { useToastStore } from '@/lib/stores/toast-store';
import { useTranslate } from '@/lib/i18n';

export default function DashboardPage() {
  const router = useRouter();
  const [users, setUsers] = useState<AdminUser[]>([]);
  const [approvals, setApprovals] = useState<ApprovalRequest[]>([]);
  const [devices, setDevices] = useState<DeviceHealth[]>([]);
  const [auditEntries, setAuditEntries] = useState<AuditEntry[]>([]);
  const [policies, setPolicies] = useState<PolicyRule[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [lastUpdated, setLastUpdated] = useState<Date | null>(null);
  const [issues, setIssues] = useState<Record<string, string>>({});
  const notify = useToastStore((s) => s.notify);
  const t = useTranslate();

  const fetchDashboardData = useCallback(async () => {
    const sections = [
      { key: 'users', url: '/api/users', field: 'users', apply: setUsers },
      { key: 'approvals', url: '/api/approvals', field: 'approvals', apply: setApprovals },
      { key: 'devices', url: '/api/devices', field: 'devices', apply: setDevices },
      { key: 'audit', url: '/api/audit?pageSize=5', field: 'entries', apply: setAuditEntries },
      { key: 'policies', url: '/api/policies', field: 'policies', apply: setPolicies },
    ];
    const failures: Record<string, string> = {};
    await Promise.allSettled(sections.map(async (section) => {
      try {
        const response = await fetch(section.url);
        if (!response.ok) throw new Error(response.status === 403 ? 'You do not have access to this section.' : `Request failed (${response.status}).`);
        const data = await response.json();
        if (!Array.isArray(data[section.field])) throw new Error('The server returned an invalid response.');
        section.apply(data[section.field]);
      } catch (e) { failures[section.key] = e instanceof Error ? e.message : 'Could not load this section.'; }
    }));
    setIssues(failures);
    if (!Object.keys(failures).length) setLastUpdated(new Date());
    return Object.keys(failures).length;
  }, []);

  useEffect(() => {
    fetchDashboardData()
      .catch(() => {})
      .finally(() => setLoading(false));
  }, [fetchDashboardData]);

  const handleRefresh = useCallback(async () => {
    setRefreshing(true);
    try {
      const failures = await fetchDashboardData();
      if (failures) notify(`${failures} dashboard section(s) could not refresh. Other sections are up to date.`, 'error');
      else notify(t('dashboard.refreshSuccess'), 'success');
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Refresh failed';
      notify(t('dashboard.refreshError', { message }), 'error');
    } finally {
      setRefreshing(false);
    }
  }, [fetchDashboardData, notify, t]);

  if (loading) {
    return (
      <div>
        <PageHeader title="Dashboard" description="Overview of your organization's file operations" />
        <div className="flex items-center justify-center py-20">
          <div className="animate-spin h-8 w-8 border-4 border-primary border-t-transparent rounded-full" />
        </div>
      </div>
    );
  }

  const activeUsers = users.filter(u => u.isActive).length;
  const pendingApprovals = approvals.filter(a => a.state === 'pending').length;
  const onlineDevices = devices.filter(d => d.status === 'online').length;
  const nonCompliantDevices = devices.filter(d => !d.policyCompliance.isCompliant).length;
  const activePolicies = policies.filter(p => p.isActive).length;
  const recentAuditEntries = auditEntries.slice(0, 5);

  return (
    <div>
      <PageHeader
        title={t('dashboard.title')}
        description={
          lastUpdated
            ? `${t('dashboard.description')} · ${t('dashboard.updatedAt', { time: formatRelativeTime(lastUpdated.toISOString()) })}`
            : t('dashboard.description')
        }
        actions={
          <Button
            variant="outline"
            size="sm"
            onClick={handleRefresh}
            loading={refreshing}
            aria-label={t('dashboard.refreshAria')}
          >
            <RefreshCw size={14} aria-hidden="true" /> {t('common.refresh')}
          </Button>
        }
      />

      {Object.keys(issues).length > 0 && <div role="alert" className="mb-4 rounded-md border border-error/20 bg-error-bg p-3 text-sm text-error-foreground">
        Some sections could not refresh. Their data is unavailable or last known; other sections remain usable.
        <ul className="mt-2">{Object.entries(issues).map(([key, message]) => <li key={key} className="capitalize">{key}: {message}</li>)}</ul>
        <Button variant="outline" size="sm" onClick={handleRefresh} loading={refreshing}>Retry unavailable sections</Button>
      </div>}
      {/* Stats Grid */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4 mb-6">
        <StatCard
          title="Active Users"
          value={issues.users ? "Unavailable" : activeUsers}
          subtitle={issues.users ? "Could not refresh users" : `${users.length} total`}
          icon={Users}
          variant="info"
        />
        <StatCard
          title="Pending Approvals"
          value={issues.approvals ? "Unavailable" : pendingApprovals}
          subtitle={issues.approvals ? "Could not refresh approvals" : pendingApprovals ? "Requires attention" : "No pending reviews"}
          icon={CheckSquare}
          variant={issues.approvals ? 'default' : pendingApprovals > 0 ? 'warning' : 'success'}
        />
        <StatCard
          title="Online Devices"
          value={issues.devices ? "Unavailable" : `${onlineDevices}/${devices.length}`}
          subtitle={issues.devices ? "Could not refresh device health" : devices.length === 0 ? 'No devices enrolled' : nonCompliantDevices > 0 ? `${nonCompliantDevices} non-compliant` : 'All compliant'}
          icon={Monitor}
          variant={issues.devices || devices.length === 0 ? 'default' : nonCompliantDevices > 0 ? 'warning' : 'success'}
        />
        <StatCard
          title="Active Policies"
          value={issues.policies ? "Unavailable" : activePolicies}
          subtitle={issues.policies ? "Could not refresh policies" : "Across all domains"}
          icon={Shield}
          variant="default"
        />
      </div>

      {/* Two column layout */}
      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* Pending Approvals */}
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <CheckSquare size={18} />
              Pending Approvals
            </CardTitle>
            <Button variant="ghost" size="sm" onClick={() => router.push("/approvals")}>
              View all <ArrowUpRight size={14} />
            </Button>
          </CardHeader>
          <CardContent>
            {approvals
              .filter(a => a.state === 'pending')
              .map((approval) => (
                <div
                  key={approval.id}
                  className="flex items-start gap-3 py-3 border-b border-border last:border-0"
                >
                  <div className="shrink-0 p-2 rounded-lg bg-warning-bg">
                    <Clock size={16} className="text-warning-foreground" />
                  </div>
                  <div className="flex-1 min-w-0">
                    <p className="text-sm font-medium text-foreground truncate">
                      {approval.operation}
                    </p>
                    <p className="text-xs text-foreground-secondary mt-0.5">
                      {approval.requestedByName} - {formatBytes(approval.totalSize)} ({approval.fileCount} files)
                    </p>
                    <p className="text-xs text-foreground-tertiary mt-0.5">
                      {formatRelativeTime(approval.requestedAt)}
                    </p>
                  </div>
                  <Badge variant="warning">Pending</Badge>
                </div>
              ))}
            {!issues.approvals && pendingApprovals === 0 && (
              <p className="text-sm text-foreground-secondary text-center py-6">
                No pending approvals
              </p>
            )}
          </CardContent>
        </Card>

        {/* Recent Audit Activity */}
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <ScrollText size={18} />
              Recent Activity
            </CardTitle>
            <Button variant="ghost" size="sm" onClick={() => router.push("/audit")}>
              View all <ArrowUpRight size={14} />
            </Button>
          </CardHeader>
          <CardContent>
            {recentAuditEntries.length > 0 ? recentAuditEntries.map((entry) => (
              <div
                key={entry.id}
                className="flex items-start gap-3 py-3 border-b border-border last:border-0"
              >
                <div className={`shrink-0 p-2 rounded-lg ${
                  entry.severity === 'warning' ? 'bg-warning-bg' :
                  entry.severity === 'error' ? 'bg-error-bg' :
                  entry.severity === 'critical' ? 'bg-error-bg' :
                  'bg-info-bg'
                }`}>
                  {entry.severity === 'warning' || entry.severity === 'error' ? (
                    <AlertTriangle size={16} className={entry.severity === 'warning' ? 'text-warning-foreground' : 'text-error-foreground'} />
                  ) : (
                    <Activity size={16} className="text-info-foreground" />
                  )}
                </div>
                <div className="flex-1 min-w-0">
                  <p className="text-sm text-foreground truncate">{entry.description}</p>
                  <p className="text-xs text-foreground-tertiary mt-0.5">
                    {entry.userName} - {formatRelativeTime(entry.timestamp)}
                  </p>
                </div>
                <Badge variant={
                  entry.severity === 'warning' ? 'warning' :
                  entry.severity === 'error' ? 'error' :
                  entry.severity === 'critical' ? 'error' :
                  'info'
                }>
                  {entry.eventType.split('.')[1]}
                </Badge>
              </div>
            )) : (
              <p className="text-sm text-foreground-secondary text-center py-6">
                {issues.audit ? "Activity unavailable. Retry to refresh." : "No recent activity"}
              </p>
            )}
          </CardContent>
        </Card>

        {/* Device Health Summary */}
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Monitor size={18} />
              Device Health
            </CardTitle>
            <Button variant="ghost" size="sm" onClick={() => router.push("/devices")}>
              View fleet <ArrowUpRight size={14} />
            </Button>
          </CardHeader>
          <CardContent>
            {devices.length > 0 ? devices.map((device) => (
              <div
                key={device.id}
                className="flex items-center gap-3 py-3 border-b border-border last:border-0"
              >
                <div className={`w-2.5 h-2.5 rounded-full shrink-0 ${
                  device.status === 'online' ? 'bg-success' :
                  device.status === 'degraded' ? 'bg-warning' :
                  device.status === 'error' ? 'bg-error' :
                  'bg-foreground-disabled'
                }`} />
                <div className="flex-1 min-w-0">
                  <p className="text-sm font-medium text-foreground">{device.deviceName}</p>
                  <p className="text-xs text-foreground-tertiary">
                    {device.userName} - {device.os} - v{device.clientVersion}
                  </p>
                </div>
                <div className="text-right">
                  <Badge variant={
                    device.status === 'online' ? 'success' :
                    device.status === 'degraded' ? 'warning' :
                    device.status === 'error' ? 'error' :
                    'default'
                  }>
                    {device.status}
                  </Badge>
                  <p className="text-xs text-foreground-tertiary mt-1">
                    {formatRelativeTime(device.lastSeenAt)}
                  </p>
                </div>
              </div>
            )) : (
              <p className="text-sm text-foreground-secondary text-center py-6">
                {issues.devices ? "Device health unavailable. Retry to refresh." : "No devices registered"}
              </p>
            )}
          </CardContent>
        </Card>

        {/* Policy Overview */}
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Shield size={18} />
              Active Policies
            </CardTitle>
            <Button variant="ghost" size="sm" onClick={() => router.push("/policies")}>
              Manage <ArrowUpRight size={14} />
            </Button>
          </CardHeader>
          <CardContent>
            {policies.filter(p => p.isActive).length > 0 ? policies
              .filter(p => p.isActive)
              .slice(0, 5)
              .map((policy) => (
                <div
                  key={policy.id}
                  className="flex items-center gap-3 py-3 border-b border-border last:border-0"
                >
                  <div className="flex-1 min-w-0">
                    <p className="text-sm font-medium text-foreground">{policy.name}</p>
                    <p className="text-xs text-foreground-tertiary">{policy.description}</p>
                  </div>
                  <Badge variant={
                    policy.enforcementMode === 'enforce' ? 'error' :
                    policy.enforcementMode === 'warn' ? 'warning' :
                    'default'
                  }>
                    {policy.enforcementMode}
                  </Badge>
                </div>
              )) : (
              <p className="text-sm text-foreground-secondary text-center py-6">
                {issues.policies ? "Policies unavailable. Retry to refresh." : "No active policies"}
              </p>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
