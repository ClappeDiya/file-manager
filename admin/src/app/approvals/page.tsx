'use client';

import React, { useState, useEffect } from 'react';
import {
  CheckCircle, XCircle, Clock, AlertTriangle, ArrowRight,
  FileText, CheckSquare,
} from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { StatCard } from '@/components/ui/stat-card';
import { SearchInput } from '@/components/ui/search-input';
import { Tabs } from '@/components/ui/tabs';
import { Dialog, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from '@/components/ui/dialog';
import { type ApprovalRequest, type ApprovalState, APPROVAL_STATE_LABELS, APPROVAL_TRIGGER_LABELS, APPROVAL_EXPIRY_HOURS } from '@/lib/types/approvals';
import { formatRelativeTime, formatBytes, formatDateTime, formatDuration } from '@/lib/utils/format';
import { useAuthStore } from '@/lib/stores/auth-store';

export default function ApprovalsPage() {
  const [approvals, setApprovals] = useState<ApprovalRequest[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState('pending');
  const [searchQuery, setSearchQuery] = useState('');
  const [triggerFilter, setTriggerFilter] = useState('');
  const [selectedApproval, setSelectedApproval] = useState<ApprovalRequest | null>(null);
  const [showReviewDialog, setShowReviewDialog] = useState(false);
  const [reviewComment, setReviewComment] = useState('');
  const [avgResponseTime] = useState(0);
  const currentUser = useAuthStore((s) => s.user);

  useEffect(() => {
    fetch('/api/approvals')
      .then((res) => {
        if (!res.ok) throw new Error('Failed to fetch approvals');
        return res.json();
      })
      .then((data) => {
        setApprovals(data.approvals || []);
        setLoading(false);
      })
      .catch((err) => { setError(err instanceof Error ? err.message : String(err)); setLoading(false); });
  }, []);

  // Auto-expire approvals past 48 hours
  useEffect(() => {
    const interval = setInterval(() => {
      const now = new Date();
      setApprovals(prev =>
        prev.map(a => {
          if (a.state === 'pending' && new Date(a.expiresAt) <= now) {
            return { ...a, state: 'expired' as ApprovalState };
          }
          return a;
        })
      );
    }, 60000); // check every minute
    return () => clearInterval(interval);
  }, []);

  const pendingCount = approvals.filter(a => a.state === 'pending').length;
  const approvedCount = approvals.filter(a => a.state === 'approved').length;
  const deniedCount = approvals.filter(a => a.state === 'denied').length;
  const expiredCount = approvals.filter(a => a.state === 'expired').length;

  const filteredApprovals = approvals.filter((a) => {
    const matchesTab = activeTab === 'all' || a.state === activeTab;
    const matchesSearch = !searchQuery ||
      a.operation.toLowerCase().includes(searchQuery.toLowerCase()) ||
      a.requestedByName.toLowerCase().includes(searchQuery.toLowerCase()) ||
      a.reason.toLowerCase().includes(searchQuery.toLowerCase());
    const matchesTrigger = !triggerFilter || a.trigger === triggerFilter;
    return matchesTab && matchesSearch && matchesTrigger;
  });

  const handleApprove = async () => {
    if (!selectedApproval || !currentUser) return;
    try {
      const res = await fetch('/api/approvals', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          id: selectedApproval.id,
          action: 'approve',
          reviewedBy: currentUser.id,
          reviewedByName: currentUser.name,
          reviewComment,
        }),
      });
      if (!res.ok) throw new Error('Failed to approve');
      const data = await res.json();
      setApprovals(approvals.map(a => a.id === selectedApproval.id ? data.approval : a));
    } catch {
      // Optimistic update fallback
      setApprovals(approvals.map(a =>
        a.id === selectedApproval.id
          ? { ...a, state: 'approved' as ApprovalState, reviewedBy: currentUser.id, reviewedByName: currentUser.name, reviewedAt: new Date().toISOString(), reviewComment }
          : a
      ));
    }
    setShowReviewDialog(false);
    setSelectedApproval(null);
    setReviewComment('');
  };

  const handleDeny = async () => {
    if (!selectedApproval || !currentUser) return;
    try {
      const res = await fetch('/api/approvals', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          id: selectedApproval.id,
          action: 'deny',
          reviewedBy: currentUser.id,
          reviewedByName: currentUser.name,
          reviewComment,
        }),
      });
      if (!res.ok) throw new Error('Failed to deny');
      const data = await res.json();
      setApprovals(approvals.map(a => a.id === selectedApproval.id ? data.approval : a));
    } catch {
      setApprovals(approvals.map(a =>
        a.id === selectedApproval.id
          ? { ...a, state: 'denied' as ApprovalState, reviewedBy: currentUser.id, reviewedByName: currentUser.name, reviewedAt: new Date().toISOString(), reviewComment }
          : a
      ));
    }
    setShowReviewDialog(false);
    setSelectedApproval(null);
    setReviewComment('');
  };

  const getTimeRemaining = (expiresAt: string): string => {
    const now = new Date();
    const expires = new Date(expiresAt);
    const diffMs = expires.getTime() - now.getTime();
    if (diffMs <= 0) return 'Expired';
    const hours = Math.floor(diffMs / (1000 * 60 * 60));
    const mins = Math.floor((diffMs % (1000 * 60 * 60)) / (1000 * 60));
    return `${hours}h ${mins}m remaining`;
  };

  if (loading) {
    return (
      <div>
        <PageHeader title="Approval Workflows" description="Review and manage approval requests" />
        <div className="flex items-center justify-center py-20">
          <div className="animate-spin h-8 w-8 border-4 border-primary border-t-transparent rounded-full" />
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div>
        <PageHeader title="Approval Workflows" description="Review and manage approval requests" />
        <div className="rounded-md bg-error-bg border border-error/20 px-4 py-3 mt-4">
          <p className="text-sm text-error">Error: {error}</p>
        </div>
      </div>
    );
  }

  return (
    <div>
      <PageHeader
        title="Approval Workflows"
        description={`Review and manage approval requests. Requests auto-expire after ${APPROVAL_EXPIRY_HOURS} hours.`}
      />

      {/* Stats */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-5 gap-4 mb-6">
        <StatCard title="Pending" value={pendingCount} icon={Clock} variant="warning" />
        <StatCard title="Approved" value={approvedCount} icon={CheckCircle} variant="success" />
        <StatCard title="Denied" value={deniedCount} icon={XCircle} variant="error" />
        <StatCard title="Expired" value={expiredCount} icon={AlertTriangle} variant="default" />
        <StatCard
          title="Avg Response"
          value={formatDuration(avgResponseTime)}
          icon={CheckSquare}
          variant="info"
        />
      </div>

      <Tabs
        tabs={[
          { id: 'pending', label: 'Pending', count: pendingCount },
          { id: 'approved', label: 'Approved', count: approvedCount },
          { id: 'denied', label: 'Denied', count: deniedCount },
          { id: 'expired', label: 'Expired', count: expiredCount },
          { id: 'all', label: 'All', count: approvals.length },
        ]}
        activeTab={activeTab}
        onTabChange={setActiveTab}
      >
        <div className="pt-4">
          {/* Filters */}
          <div className="flex flex-col sm:flex-row gap-3 mb-4">
            <SearchInput
              placeholder="Search approvals..."
              onSearch={setSearchQuery}
              className="sm:w-72"
            />
            <Select
              options={[
                { value: '', label: 'All Triggers' },
                ...Object.entries(APPROVAL_TRIGGER_LABELS).map(([v, l]) => ({ value: v, label: l })),
              ]}
              value={triggerFilter}
              onChange={(e) => setTriggerFilter(e.target.value)}
              className="sm:w-56"
            />
          </div>

          {/* Approval Cards */}
          <div className="space-y-3">
            {filteredApprovals.map((approval) => (
              <Card key={approval.id} className="hover:border-border-hover transition-colors">
                <div className="flex flex-col lg:flex-row lg:items-center gap-4">
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2 mb-1">
                      <Badge variant={
                        approval.state === 'pending' ? 'warning' :
                        approval.state === 'approved' ? 'success' :
                        approval.state === 'denied' ? 'error' : 'default'
                      }>
                        {APPROVAL_STATE_LABELS[approval.state]}
                      </Badge>
                      <Badge variant="outline">{APPROVAL_TRIGGER_LABELS[approval.trigger]}</Badge>
                      {approval.state === 'pending' && (
                        <span className="text-xs text-warning font-medium">
                          {getTimeRemaining(approval.expiresAt)}
                        </span>
                      )}
                    </div>

                    <h3 className="text-sm font-semibold text-foreground">{approval.operation}</h3>

                    <div className="flex items-center gap-1 text-xs text-foreground-secondary mt-1">
                      <span>{approval.source}</span>
                      <ArrowRight size={12} />
                      <span>{approval.destination}</span>
                    </div>

                    <p className="text-xs text-foreground-tertiary mt-1">
                      {approval.fileCount} files ({formatBytes(approval.totalSize)}) - Requested by {approval.requestedByName}
                    </p>
                    <p className="text-xs text-foreground-secondary mt-1 italic">
                      &quot;{approval.reason}&quot;
                    </p>

                    {approval.reviewedByName && (
                      <p className="text-xs text-foreground-tertiary mt-2">
                        {approval.state === 'approved' ? 'Approved' : 'Denied'} by {approval.reviewedByName} on {formatDateTime(approval.reviewedAt!)}
                        {approval.reviewComment && ` - "${approval.reviewComment}"`}
                      </p>
                    )}
                  </div>

                  <div className="flex items-center gap-2 shrink-0">
                    <span className="text-xs text-foreground-tertiary">
                      {formatRelativeTime(approval.requestedAt)}
                    </span>
                    {approval.state === 'pending' && (
                      <>
                        <Button
                          variant="primary"
                          size="sm"
                          onClick={() => { setSelectedApproval(approval); setShowReviewDialog(true); }}
                        >
                          <CheckCircle size={14} /> Approve
                        </Button>
                        <Button
                          variant="danger"
                          size="sm"
                          onClick={() => { setSelectedApproval(approval); setShowReviewDialog(true); }}
                        >
                          <XCircle size={14} /> Deny
                        </Button>
                      </>
                    )}
                    {approval.state !== 'pending' && (
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => { setSelectedApproval(approval); setShowReviewDialog(true); }}
                      >
                        <FileText size={14} /> Details
                      </Button>
                    )}
                  </div>
                </div>
              </Card>
            ))}

            {filteredApprovals.length === 0 && (
              <div className="text-center py-12 text-foreground-secondary">
                <CheckSquare size={32} className="mx-auto mb-3 text-foreground-tertiary" />
                <p className="text-sm">No approval requests match your filters</p>
              </div>
            )}
          </div>
        </div>
      </Tabs>

      {/* Review Dialog */}
      <Dialog
        open={showReviewDialog}
        onClose={() => { setShowReviewDialog(false); setSelectedApproval(null); setReviewComment(''); }}
        className="max-w-xl"
      >
        <DialogHeader>
          <DialogTitle>
            {selectedApproval?.state === 'pending' ? 'Review Approval Request' : 'Approval Details'}
          </DialogTitle>
          <DialogDescription>
            {selectedApproval?.operation} - {selectedApproval?.requestedByName}
          </DialogDescription>
        </DialogHeader>
        {selectedApproval && (
          <div className="space-y-4">
            <div className="grid grid-cols-2 gap-3 text-sm">
              <div>
                <p className="text-foreground-tertiary">Trigger</p>
                <p className="font-medium">{APPROVAL_TRIGGER_LABELS[selectedApproval.trigger]}</p>
              </div>
              <div>
                <p className="text-foreground-tertiary">Policy</p>
                <p className="font-medium">{selectedApproval.triggeringPolicy}</p>
              </div>
              <div>
                <p className="text-foreground-tertiary">Source</p>
                <p className="font-medium">{selectedApproval.source}</p>
              </div>
              <div>
                <p className="text-foreground-tertiary">Destination</p>
                <p className="font-medium">{selectedApproval.destination}</p>
              </div>
              <div>
                <p className="text-foreground-tertiary">Files</p>
                <p className="font-medium">{selectedApproval.fileCount} files ({formatBytes(selectedApproval.totalSize)})</p>
              </div>
              <div>
                <p className="text-foreground-tertiary">Expires</p>
                <p className="font-medium">{formatDateTime(selectedApproval.expiresAt)}</p>
              </div>
            </div>
            <div>
              <p className="text-sm text-foreground-tertiary">Reason</p>
              <p className="text-sm mt-1">{selectedApproval.reason}</p>
            </div>
            {selectedApproval.state === 'pending' && (
              <Input
                label="Review Comment (optional)"
                placeholder="Add a comment for the requester..."
                value={reviewComment}
                onChange={(e) => setReviewComment(e.target.value)}
              />
            )}
            {selectedApproval.reviewComment && selectedApproval.state !== 'pending' && (
              <div>
                <p className="text-sm text-foreground-tertiary">Reviewer Comment</p>
                <p className="text-sm mt-1">{selectedApproval.reviewComment}</p>
              </div>
            )}
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => { setShowReviewDialog(false); setSelectedApproval(null); setReviewComment(''); }}>
            {selectedApproval?.state === 'pending' ? 'Cancel' : 'Close'}
          </Button>
          {selectedApproval?.state === 'pending' && (
            <>
              <Button variant="danger" onClick={handleDeny}>
                <XCircle size={14} /> Deny
              </Button>
              <Button onClick={handleApprove}>
                <CheckCircle size={14} /> Approve
              </Button>
            </>
          )}
        </DialogFooter>
      </Dialog>
    </div>
  );
}
