'use client';

import React, { useState, useEffect } from 'react';
import {
  Users, Monitor, Shield, CheckSquare, ScrollText,
  AlertTriangle, Activity, ArrowUpRight, Clock,
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

export default function DashboardPage() {
  const [users, setUsers] = useState<AdminUser[]>([]);
  const [approvals, setApprovals] = useState<ApprovalRequest[]>([]);
  const [devices, setDevices] = useState<DeviceHealth[]>([]);
  const [auditEntries, setAuditEntries] = useState<AuditEntry[]>([]);
  const [policies, setPolicies] = useState<PolicyRule[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    async function fetchDashboardData() {
      try {
        const [usersRes, approvalsRes, devicesRes, auditRes, policiesRes] = await Promise.all([
          fetch('/api/users'),
          fetch('/api/approvals'),
          fetch('/api/devices'),
          fetch('/api/audit?pageSize=5'),
          fetch('/api/policies'),
        ]);

        if (!usersRes.ok || !approvalsRes.ok || !devicesRes.ok || !auditRes.ok || !policiesRes.ok) {
          throw new Error('Failed to fetch dashboard data');
        }

        const [usersData, approvalsData, devicesData, auditData, policiesData] = await Promise.all([
          usersRes.json(),
          approvalsRes.json(),
          devicesRes.json(),
          auditRes.json(),
          policiesRes.json(),
        ]);

        setUsers(usersData.users || []);
        setApprovals(approvalsData.approvals || []);
        setDevices(devicesData.devices || []);
        setAuditEntries(auditData.entries || []);
        setPolicies(policiesData.policies || []);
      } catch (err) {
        setError(err instanceof Error ? err.message : 'Failed to load dashboard');
      } finally {
        setLoading(false);
      }
    }

    fetchDashboardData();
  }, []);

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

  if (error) {
    return (
      <div>
        <PageHeader title="Dashboard" description="Overview of your organization's file operations" />
        <div className="rounded-md bg-error-bg border border-error/20 px-4 py-3 mt-4">
          <p className="text-sm text-error">Error: {error}</p>
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
        title="Dashboard"
        description="Overview of your organization's file operations"
      />

      {/* Stats Grid */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4 mb-6">
        <StatCard
          title="Active Users"
          value={activeUsers}
          subtitle={`${users.length} total`}
          icon={Users}
          variant="info"
          trend={{ value: 12, label: 'from last month' }}
        />
        <StatCard
          title="Pending Approvals"
          value={pendingApprovals}
          subtitle="Requires attention"
          icon={CheckSquare}
          variant={pendingApprovals > 0 ? 'warning' : 'success'}
        />
        <StatCard
          title="Online Devices"
          value={`${onlineDevices}/${devices.length}`}
          subtitle={nonCompliantDevices > 0 ? `${nonCompliantDevices} non-compliant` : 'All compliant'}
          icon={Monitor}
          variant={nonCompliantDevices > 0 ? 'warning' : 'success'}
        />
        <StatCard
          title="Active Policies"
          value={activePolicies}
          subtitle="Across all domains"
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
            <Button variant="ghost" size="sm">
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
                    <Clock size={16} className="text-warning" />
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
            {pendingApprovals === 0 && (
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
            <Button variant="ghost" size="sm">
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
                    <AlertTriangle size={16} className={entry.severity === 'warning' ? 'text-warning' : 'text-error'} />
                  ) : (
                    <Activity size={16} className="text-info" />
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
                No recent activity
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
            <Button variant="ghost" size="sm">
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
                No devices registered
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
            <Button variant="ghost" size="sm">
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
                No active policies
              </p>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
