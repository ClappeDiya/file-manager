'use client';

import React, { useState, useEffect } from 'react';
import { Monitor, Wifi, WifiOff, AlertTriangle, Shield, Cpu, HardDrive, MemoryStick } from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card, CardContent } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { StatCard } from '@/components/ui/stat-card';
import { SearchInput } from '@/components/ui/search-input';
import { Select } from '@/components/ui/select';
import { Table, TableHeader, TableBody, TableRow, TableHead, TableCell } from '@/components/ui/table';
import { formatRelativeTime } from '@/lib/utils/format';
import type { DeviceHealth } from '@/lib/types/audit';

export default function DevicesPage() {
  const [devices, setDevices] = useState<DeviceHealth[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState('');
  const [statusFilter, setStatusFilter] = useState('');
  const [selectedDevice, setSelectedDevice] = useState<DeviceHealth | null>(null);
  const [view, setView] = useState<'fleet' | 'detail'>('fleet');

  useEffect(() => {
    fetch('/api/devices')
      .then((res) => {
        if (!res.ok) throw new Error('Failed to fetch devices');
        return res.json();
      })
      .then((data) => { setDevices(data.devices || []); setLoading(false); })
      .catch((err) => { setError(err instanceof Error ? err.message : String(err)); setLoading(false); });
  }, []);

  const filteredDevices = devices.filter((device) => {
    const matchesSearch = !searchQuery ||
      device.deviceName.toLowerCase().includes(searchQuery.toLowerCase()) ||
      device.userName.toLowerCase().includes(searchQuery.toLowerCase());
    const matchesStatus = !statusFilter || device.status === statusFilter;
    return matchesSearch && matchesStatus;
  });

  const onlineCount = devices.filter(d => d.status === 'online').length;
  const degradedCount = devices.filter(d => d.status === 'degraded').length;
  const offlineCount = devices.filter(d => d.status === 'offline').length;
  const nonCompliantCount = devices.filter(d => !d.policyCompliance.isCompliant).length;

  const openDetail = (device: DeviceHealth) => {
    setSelectedDevice(device);
    setView('detail');
  };

  if (loading) {
    return (
      <div>
        <PageHeader title="Devices" description="Monitor device health and policy compliance" />
        <div className="flex items-center justify-center py-20">
          <div className="animate-spin h-8 w-8 border-4 border-primary border-t-transparent rounded-full" />
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div>
        <PageHeader title="Devices" description="Monitor device health and policy compliance" />
        <div className="rounded-md bg-error-bg border border-error/20 px-4 py-3 mt-4">
          <p className="text-sm text-error">Error: {error}</p>
        </div>
      </div>
    );
  }

  return (
    <div>
      <PageHeader
        title="Devices"
        description="Monitor device health, client versions, and policy compliance across your fleet"
        actions={
          view === 'detail' ? (
            <Button variant="outline" size="sm" onClick={() => { setView('fleet'); setSelectedDevice(null); }}>
              Back to Fleet View
            </Button>
          ) : undefined
        }
      />

      {view === 'fleet' ? (
        <>
          {/* Stats */}
          <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-4 gap-4 mb-6">
            <StatCard title="Online" value={onlineCount} icon={Wifi} variant="success" />
            <StatCard title="Degraded" value={degradedCount} icon={AlertTriangle} variant="warning" />
            <StatCard title="Offline" value={offlineCount} icon={WifiOff} variant="default" />
            <StatCard title="Non-Compliant" value={nonCompliantCount} icon={Shield} variant="error" />
          </div>

          {/* Filters */}
          <div className="flex flex-col sm:flex-row gap-3 mb-4">
            <SearchInput
              placeholder="Search by device or user..."
              onSearch={setSearchQuery}
              className="sm:w-72"
            />
            <Select
              options={[
                { value: '', label: 'All Statuses' },
                { value: 'online', label: 'Online' },
                { value: 'degraded', label: 'Degraded' },
                { value: 'offline', label: 'Offline' },
                { value: 'error', label: 'Error' },
              ]}
              value={statusFilter}
              onChange={(e) => setStatusFilter(e.target.value)}
              className="sm:w-48"
            />
          </div>

          {/* Fleet Table */}
          <Card padding="none">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Device</TableHead>
                  <TableHead>User</TableHead>
                  <TableHead>Version</TableHead>
                  <TableHead className="hidden md:table-cell">OS</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead className="hidden lg:table-cell">Compliance</TableHead>
                  <TableHead className="hidden lg:table-cell">Last Seen</TableHead>
                  <TableHead className="w-20">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {filteredDevices.map((device) => (
                  <TableRow key={device.id}>
                    <TableCell>
                      <div className="flex items-center gap-2">
                        <Monitor size={16} className="text-foreground-secondary shrink-0" />
                        <span className="text-sm font-medium">{device.deviceName}</span>
                      </div>
                    </TableCell>
                    <TableCell className="text-sm text-foreground-secondary">{device.userName}</TableCell>
                    <TableCell>
                      <span className="text-sm font-mono">v{device.clientVersion}</span>
                    </TableCell>
                    <TableCell className="hidden md:table-cell text-sm text-foreground-secondary">
                      {device.os} {device.osVersion}
                    </TableCell>
                    <TableCell>
                      <Badge variant={
                        device.status === 'online' ? 'success' :
                        device.status === 'degraded' ? 'warning' :
                        device.status === 'error' ? 'error' : 'default'
                      }>
                        {device.status}
                      </Badge>
                    </TableCell>
                    <TableCell className="hidden lg:table-cell">
                      <Badge variant={device.policyCompliance.isCompliant ? 'success' : 'error'}>
                        {device.policyCompliance.isCompliant ? 'Compliant' : `${device.policyCompliance.violations.length} violations`}
                      </Badge>
                    </TableCell>
                    <TableCell className="hidden lg:table-cell text-sm text-foreground-secondary">
                      {formatRelativeTime(device.lastSeenAt)}
                    </TableCell>
                    <TableCell>
                      <Button variant="ghost" size="sm" onClick={() => openDetail(device)}>
                        Details
                      </Button>
                    </TableCell>
                  </TableRow>
                ))}
                {filteredDevices.length === 0 && (
                  <TableRow>
                    <TableCell colSpan={8} className="text-center py-8 text-foreground-secondary">
                      No devices found
                    </TableCell>
                  </TableRow>
                )}
              </TableBody>
            </Table>
          </Card>
        </>
      ) : selectedDevice ? (
        /* Device Detail View */
        <div className="grid gap-6">
          {/* Device Info */}
          <Card>
            <div className="flex items-start justify-between">
              <div>
                <h2 className="text-xl font-bold text-foreground">{selectedDevice.deviceName}</h2>
                <p className="text-sm text-foreground-secondary mt-1">
                  {selectedDevice.userName} - {selectedDevice.os} {selectedDevice.osVersion}
                </p>
              </div>
              <Badge
                variant={selectedDevice.status === 'online' ? 'success' : selectedDevice.status === 'degraded' ? 'warning' : 'default'}
                className="text-sm"
              >
                {selectedDevice.status}
              </Badge>
            </div>
          </Card>

          <div className="grid grid-cols-1 md:grid-cols-3 gap-4">
            <StatCard
              title="CPU Usage"
              value={`${selectedDevice.cpuUsage ?? 0}%`}
              icon={Cpu}
              variant={selectedDevice.cpuUsage && selectedDevice.cpuUsage > 80 ? 'error' : 'default'}
            />
            <StatCard
              title="Memory Usage"
              value={`${selectedDevice.memoryUsage ?? 0}%`}
              icon={MemoryStick}
              variant={selectedDevice.memoryUsage && selectedDevice.memoryUsage > 80 ? 'warning' : 'default'}
            />
            <StatCard
              title="Disk Usage"
              value={`${selectedDevice.diskUsage ?? 0}%`}
              icon={HardDrive}
              variant={selectedDevice.diskUsage && selectedDevice.diskUsage > 85 ? 'error' : 'default'}
            />
          </div>

          {/* Compliance */}
          <Card>
            <CardContent>
              <h3 className="text-lg font-semibold mb-4">Policy Compliance</h3>
              {selectedDevice.policyCompliance.isCompliant ? (
                <div className="flex items-center gap-2 text-success">
                  <Shield size={18} />
                  <span className="text-sm font-medium">Fully Compliant</span>
                </div>
              ) : (
                <div className="space-y-3">
                  {selectedDevice.policyCompliance.violations.map((v, i) => (
                    <div key={i} className="flex items-start gap-3 p-3 rounded-lg bg-error-bg border border-error/20">
                      <AlertTriangle size={16} className="text-error shrink-0 mt-0.5" />
                      <div>
                        <p className="text-sm font-medium text-foreground">{v.policyName}</p>
                        <p className="text-xs text-foreground-secondary mt-0.5">{v.description}</p>
                        <Badge variant="error" className="mt-1">{v.severity}</Badge>
                      </div>
                    </div>
                  ))}
                </div>
              )}
            </CardContent>
          </Card>

          {/* Failure Patterns */}
          {selectedDevice.failurePatterns.length > 0 && (
            <Card>
              <CardContent>
                <h3 className="text-lg font-semibold mb-4">Failure Patterns</h3>
                <div className="space-y-3">
                  {selectedDevice.failurePatterns.map((fp, i) => (
                    <div key={i} className="flex items-start gap-3 p-3 rounded-lg bg-warning-bg border border-warning/20">
                      <AlertTriangle size={16} className="text-warning shrink-0 mt-0.5" />
                      <div>
                        <p className="text-sm font-medium text-foreground">{fp.type}</p>
                        <p className="text-xs text-foreground-secondary mt-0.5">{fp.description}</p>
                        <p className="text-xs text-foreground-tertiary mt-1">
                          Occurred {fp.count} times - Last: {formatRelativeTime(fp.lastOccurrence)}
                        </p>
                      </div>
                    </div>
                  ))}
                </div>
              </CardContent>
            </Card>
          )}
        </div>
      ) : null}
    </div>
  );
}
