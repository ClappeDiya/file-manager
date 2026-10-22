'use client';

import React, { useState, useEffect } from 'react';
import {
  Download, Shield,
  AlertTriangle, Info, AlertOctagon,
  Webhook,
} from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { SearchInput } from '@/components/ui/search-input';
import { Tabs, TabContent } from '@/components/ui/tabs';
import { Dialog, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from '@/components/ui/dialog';
import { Table, TableHeader, TableBody, TableRow, TableHead, TableCell } from '@/components/ui/table';
import {
  type AuditEntry, type AuditSeverity,
  AUDIT_EVENT_CATEGORIES,
} from '@/lib/types/audit';
import { formatDateTime, formatRelativeTime } from '@/lib/utils/format';
import type { AuditVerification } from '@/lib/db/repos/audit';
import { SiemDialog } from '@/components/audit/siem-dialog';
import { ApiRequestsLog } from '@/components/audit/api-requests';
import { ExportsPanel } from '@/components/exports/exports-panel';

export default function AuditPage() {
  const [auditEntries, setAuditEntries] = useState<AuditEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState('explorer');
  const [searchQuery, setSearchQuery] = useState('');
  const [severityFilter, setSeverityFilter] = useState('');
  const [categoryFilter, setCategoryFilter] = useState('');
  const [dateFrom, setDateFrom] = useState('');
  const [dateTo, setDateTo] = useState('');
  const [selectedEntry, setSelectedEntry] = useState<AuditEntry | null>(null);
  const [showDetailDialog, setShowDetailDialog] = useState(false);
  const [showExportDialog, setShowExportDialog] = useState(false);
  const [showSIEMDialog, setShowSIEMDialog] = useState(false);
  const [verificationResult, setVerificationResult] = useState<string | null>(null);


  useEffect(() => {
    fetch('/api/audit')
      .then((res) => {
        if (!res.ok) throw new Error('Failed to fetch audit entries');
        return res.json();
      })
      .then((data) => { setAuditEntries(data.entries || []); setLoading(false); })
      .catch((err) => { setError(err instanceof Error ? err.message : String(err)); setLoading(false); });
  }, []);

  const filteredEntries = auditEntries.filter((entry) => {
    const matchesSearch = !searchQuery ||
      entry.description.toLowerCase().includes(searchQuery.toLowerCase()) ||
      entry.userName.toLowerCase().includes(searchQuery.toLowerCase()) ||
      entry.eventType.toLowerCase().includes(searchQuery.toLowerCase());
    const matchesSeverity = !severityFilter || entry.severity === severityFilter;
    const matchesCategory = !categoryFilter ||
      (AUDIT_EVENT_CATEGORIES[categoryFilter]?.includes(entry.eventType) ?? false);
    const matchesDateFrom = !dateFrom || new Date(entry.timestamp) >= new Date(dateFrom);
    const matchesDateTo = !dateTo || new Date(entry.timestamp) <= new Date(dateTo + 'T23:59:59Z');
    return matchesSearch && matchesSeverity && matchesCategory && matchesDateFrom && matchesDateTo;
  });

  const severityIcon = (severity: AuditSeverity) => {
    switch (severity) {
      case 'info': return <Info size={14} className="text-info" />;
      case 'warning': return <AlertTriangle size={14} className="text-warning" />;
      case 'error': return <AlertOctagon size={14} className="text-error" />;
      case 'critical': return <AlertOctagon size={14} className="text-error" />;
    }
  };

  const exportParams = () => {
    const p: Record<string, string> = {};
    if (searchQuery) p.search = searchQuery;
    if (severityFilter) p.severity = severityFilter;
    if (dateFrom) p.dateFrom = dateFrom;
    if (dateTo) p.dateTo = dateTo;
    return p;
  };

  // The server recomputes the full hash chain and checks the signed checkpoints.
  const describe = (v: AuditVerification, anchors?: number) =>
    v.ok
      ? `Intact: ${v.entriesChecked} entries chained, ${v.anchorsChecked} signed checkpoint(s) valid${anchors !== undefined ? ` (${anchors} listed)` : ''}. Head #${v.headSequence} ${v.headHash.slice(0, 16)}…`
      : `TAMPERING DETECTED at entry #${v.firstBadSequence ?? '?'}: ${v.reason ?? 'chain mismatch'}`;

  const handleVerifyIntegrity = async () => {
    try {
      const res = await fetch('/api/audit/verify');
      const data = await res.json();
      if (!res.ok) throw new Error(data.error || 'Verification failed');
      setVerificationResult(describe(data.verification, data.anchors?.length));
    } catch (e) {
      setVerificationResult(e instanceof Error ? e.message : String(e));
    }
  };

  const handleAnchor = async () => {
    try {
      const res = await fetch('/api/audit/verify', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: '{}' });
      const data = await res.json();
      if (!res.ok) throw new Error(data.error || 'Could not sign a checkpoint');
      setVerificationResult(data.anchor ? `Signed checkpoint at entry #${data.anchor.lastSequence}. ${describe(data.verification)}` : describe(data.verification));
    } catch (e) {
      setVerificationResult(e instanceof Error ? e.message : String(e));
    }
  };

  if (loading) {
    return (
      <div>
        <PageHeader title="Audit Log" description="Searchable, filterable audit trail" />
        <div className="flex items-center justify-center py-20">
          <div className="animate-spin h-8 w-8 border-4 border-primary border-t-transparent rounded-full" />
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div>
        <PageHeader title="Audit Log" description="Searchable, filterable audit trail" />
        <div className="rounded-md bg-error-bg border border-error/20 px-4 py-3 mt-4">
          <p className="text-sm text-error">Error: {error}</p>
        </div>
      </div>
    );
  }

  return (
    <div>
      <PageHeader
        title="Audit Log"
        description="Searchable, filterable audit trail with immutable entries and SIEM integration"
        actions={
          <div className="flex gap-2">
            <Button variant="outline" size="sm" onClick={handleVerifyIntegrity}>
              <Shield size={14} /> Verify Integrity
            </Button>
            <Button variant="outline" size="sm" onClick={handleAnchor}>
              Sign checkpoint
            </Button>
            <Button variant="outline" size="sm" onClick={() => setShowExportDialog(true)}>
              <Download size={14} /> Export
            </Button>
            <Button variant="outline" size="sm" onClick={() => setShowSIEMDialog(true)}>
              <Webhook size={14} /> SIEM Config
            </Button>
          </div>
        }
      />

      {verificationResult && (
        <div className="mb-4 p-3 rounded-lg bg-info-bg border border-info/20">
          <p className="text-sm text-info-foreground">{verificationResult}</p>
          <button
            onClick={() => setVerificationResult(null)}
            className="text-xs text-info underline mt-1"
          >
            Dismiss
          </button>
        </div>
      )}

      <Tabs
        tabs={[
          { id: 'explorer', label: 'Audit Explorer', count: auditEntries.length },
          { id: 'categories', label: 'Event Categories' },
          { id: 'api', label: 'API requests' },
        ]}
        activeTab={activeTab}
        onTabChange={setActiveTab}
      >
        <TabContent tabId="explorer" activeTab={activeTab}>
          {/* Advanced Filters */}
          <div className="flex flex-col sm:flex-row gap-3 mb-4">
            <SearchInput
              placeholder="Search by description, user, event type..."
              onSearch={setSearchQuery}
              className="sm:w-80"
            />
            <Select
              options={[
                { value: '', label: 'All Severities' },
                { value: 'info', label: 'Info' },
                { value: 'warning', label: 'Warning' },
                { value: 'error', label: 'Error' },
                { value: 'critical', label: 'Critical' },
              ]}
              value={severityFilter}
              onChange={(e) => setSeverityFilter(e.target.value)}
              className="sm:w-40"
            />
            <Select
              options={[
                { value: '', label: 'All Categories' },
                ...Object.keys(AUDIT_EVENT_CATEGORIES).map(c => ({ value: c, label: c })),
              ]}
              value={categoryFilter}
              onChange={(e) => setCategoryFilter(e.target.value)}
              className="sm:w-48"
            />
            <Input
              type="date"
              placeholder="From"
              value={dateFrom}
              onChange={(e) => setDateFrom(e.target.value)}
              className="sm:w-40"
            />
            <Input
              type="date"
              placeholder="To"
              value={dateTo}
              onChange={(e) => setDateTo(e.target.value)}
              className="sm:w-40"
            />
          </div>

          {/* Audit Entries Table */}
          <Card padding="none">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="w-8"></TableHead>
                  <TableHead>Event</TableHead>
                  <TableHead className="hidden md:table-cell">User</TableHead>
                  <TableHead>Description</TableHead>
                  <TableHead className="hidden lg:table-cell">IP</TableHead>
                  <TableHead>Time</TableHead>
                  <TableHead className="w-20">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {filteredEntries.map((entry) => (
                  <TableRow key={entry.id}>
                    <TableCell>{severityIcon(entry.severity)}</TableCell>
                    <TableCell>
                      <Badge variant="outline" className="text-xs font-mono">
                        {entry.eventType}
                      </Badge>
                    </TableCell>
                    <TableCell className="hidden md:table-cell">
                      <div>
                        <p className="text-sm">{entry.userName}</p>
                        <p className="text-xs text-foreground-tertiary">{entry.userEmail}</p>
                      </div>
                    </TableCell>
                    <TableCell>
                      <p className="text-sm text-foreground max-w-xs truncate">{entry.description}</p>
                    </TableCell>
                    <TableCell className="hidden lg:table-cell text-sm text-foreground-secondary font-mono">
                      {entry.ipAddress}
                    </TableCell>
                    <TableCell className="text-sm text-foreground-secondary whitespace-nowrap">
                      {formatRelativeTime(entry.timestamp)}
                    </TableCell>
                    <TableCell>
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => { setSelectedEntry(entry); setShowDetailDialog(true); }}
                      >
                        Details
                      </Button>
                    </TableCell>
                  </TableRow>
                ))}
                {filteredEntries.length === 0 && (
                  <TableRow>
                    <TableCell colSpan={7} className="text-center py-8 text-foreground-secondary">
                      No audit entries found
                    </TableCell>
                  </TableRow>
                )}
              </TableBody>
            </Table>
          </Card>
        </TabContent>

        <TabContent tabId="api" activeTab={activeTab}>
          {activeTab === 'api' && <ApiRequestsLog />}
        </TabContent>

        <TabContent tabId="categories" activeTab={activeTab}>
          <div className="grid gap-4">
            {Object.entries(AUDIT_EVENT_CATEGORIES).map(([category, events]) => (
              <Card key={category}>
                <h3 className="text-sm font-semibold text-foreground mb-3">{category}</h3>
                <div className="flex flex-wrap gap-2">
                  {events.map((event) => {
                    const count = auditEntries.filter(e => e.eventType === event).length;
                    return (
                      <Badge key={event} variant={count > 0 ? 'info' : 'default'}>
                        {event} {count > 0 && `(${count})`}
                      </Badge>
                    );
                  })}
                </div>
              </Card>
            ))}
          </div>
        </TabContent>
      </Tabs>

      {/* Entry Detail Dialog */}
      <Dialog open={showDetailDialog} onClose={() => { setShowDetailDialog(false); setSelectedEntry(null); }}>
        <DialogHeader>
          <DialogTitle>Audit Entry Details</DialogTitle>
          <DialogDescription>Immutable record - {selectedEntry?.id}</DialogDescription>
        </DialogHeader>
        {selectedEntry && (
          <div className="space-y-3 text-sm">
            <div className="grid grid-cols-2 gap-3">
              <div>
                <p className="text-foreground-tertiary">Event Type</p>
                <Badge variant="outline" className="font-mono">{selectedEntry.eventType}</Badge>
              </div>
              <div>
                <p className="text-foreground-tertiary">Severity</p>
                <div className="flex items-center gap-1">
                  {severityIcon(selectedEntry.severity)}
                  <span className="capitalize">{selectedEntry.severity}</span>
                </div>
              </div>
              <div>
                <p className="text-foreground-tertiary">Timestamp</p>
                <p>{formatDateTime(selectedEntry.timestamp)}</p>
              </div>
              <div>
                <p className="text-foreground-tertiary">IP Address</p>
                <p className="font-mono">{selectedEntry.ipAddress}</p>
              </div>
              <div>
                <p className="text-foreground-tertiary">User</p>
                <p>{selectedEntry.userName} ({selectedEntry.userEmail})</p>
              </div>
              {selectedEntry.resourceType && (
                <div>
                  <p className="text-foreground-tertiary">Resource</p>
                  <p>{selectedEntry.resourceType}: {selectedEntry.resourceId}</p>
                </div>
              )}
            </div>
            <div>
              <p className="text-foreground-tertiary">Description</p>
              <p>{selectedEntry.description}</p>
            </div>
            <div>
              <p className="text-foreground-tertiary">Metadata</p>
              <pre className="text-xs bg-background-secondary rounded-md p-3 overflow-auto max-h-32 font-mono">
                {JSON.stringify(selectedEntry.metadata, null, 2)}
              </pre>
            </div>
            <div>
              <p className="text-foreground-tertiary">Integrity Hash</p>
              <p className="font-mono text-xs break-all">{selectedEntry.integrityHash}</p>
            </div>
            {selectedEntry.previousHash && (
              <div>
                <p className="text-foreground-tertiary">Previous Hash</p>
                <p className="font-mono text-xs break-all">{selectedEntry.previousHash}</p>
              </div>
            )}
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => { setShowDetailDialog(false); setSelectedEntry(null); }}>Close</Button>
        </DialogFooter>
      </Dialog>

      {/* Export Dialog */}
      <Dialog open={showExportDialog} onClose={() => setShowExportDialog(false)}>
        <DialogHeader>
          <DialogTitle>Export Audit Log</DialogTitle>
          <DialogDescription>Exports every entry matching the current filters (search, severity, dates), oldest first, with their hash-chain fields.</DialogDescription>
        </DialogHeader>
        {showExportDialog && <ExportsPanel kind="audit" formats={['csv', 'json', 'syslog']} params={exportParams()} />}
        <DialogFooter>
          <Button variant="outline" onClick={() => setShowExportDialog(false)}>Close</Button>
        </DialogFooter>
      </Dialog>

      <SiemDialog open={showSIEMDialog} onClose={() => setShowSIEMDialog(false)} />
    </div>
  );
}
