'use client';

import React, { useState, useEffect } from 'react';
import { Plus, Shield, Settings, Globe, Lock, FileType, Hash, Type, GitMerge, Trash2, Brain, Clock, RefreshCcw } from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { Dialog, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from '@/components/ui/dialog';
import { SearchInput } from '@/components/ui/search-input';
import { Table, TableHeader, TableBody, TableRow, TableHead, TableCell } from '@/components/ui/table';
import {
  type PolicyRule, type PolicyDomain, type PolicyAssignmentTarget, type PolicyEnforcementMode,
  POLICY_DOMAIN_LABELS, POLICY_DOMAIN_DESCRIPTIONS,
} from '@/lib/types/policies';
import { formatDate } from '@/lib/utils/format';
// Policy conflict detection is now handled server-side via the API

const DOMAIN_ICONS: Record<PolicyDomain, React.ElementType> = {
  allowed_connectors: Settings,
  approved_destinations: Globe,
  encryption_requirements: Lock,
  max_transfer_size: Hash,
  file_type_restrictions: FileType,
  checksum_requirements: Hash,
  naming_strictness: Type,
  sync_conflict_policies: GitMerge,
  deletion_restrictions: Trash2,
  ai_usage: Brain,
  retention: Clock,
};

export default function PoliciesPage() {
  const [searchQuery, setSearchQuery] = useState('');
  const [domainFilter, setDomainFilter] = useState('');
  const [policies, setPolicies] = useState<PolicyRule[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showCreateDialog, setShowCreateDialog] = useState(false);
  const [showDetailDialog, setShowDetailDialog] = useState(false);
  const [selectedPolicy, setSelectedPolicy] = useState<PolicyRule | null>(null);
  const [propagatingPolicies, setPropagatingPolicies] = useState<Set<string>>(new Set());

  // New policy form state
  const [newDomain, setNewDomain] = useState<PolicyDomain>('encryption_requirements');
  const [newName, setNewName] = useState('');
  const [newDescription, setNewDescription] = useState('');
  const [newTarget, setNewTarget] = useState<PolicyAssignmentTarget>('org');
  const [newTargetValue, setNewTargetValue] = useState('org_001');
  const [newMode, setNewMode] = useState<PolicyEnforcementMode>('enforce');

  useEffect(() => {
    fetch('/api/policies')
      .then((res) => {
        if (!res.ok) throw new Error('Failed to fetch policies');
        return res.json();
      })
      .then((data) => { setPolicies(data.policies || []); setLoading(false); })
      .catch((err) => { setError(err instanceof Error ? err.message : String(err)); setLoading(false); });
  }, []);

  const filteredPolicies = policies.filter((policy) => {
    const matchesSearch = !searchQuery ||
      policy.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
      policy.description.toLowerCase().includes(searchQuery.toLowerCase());
    const matchesDomain = !domainFilter || policy.domain === domainFilter;
    return matchesSearch && matchesDomain;
  });

  const domainOptions = Object.entries(POLICY_DOMAIN_LABELS).map(([value, label]) => ({ value, label }));

  const handleCreatePolicy = async () => {
    if (!newName) return;

    try {
      const res = await fetch('/api/policies', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          domain: newDomain,
          name: newName,
          description: newDescription,
          assignmentTarget: newTarget,
          assignmentValue: newTargetValue,
          enforcementMode: newMode,
          configuration: {},
        }),
      });

      if (!res.ok) throw new Error('Failed to create policy');
      const data = await res.json();

      if (data.conflicts?.length > 0) {
        console.warn('Policy conflicts detected:', data.conflicts);
      }

      setPolicies([...policies, data.policy]);
      setShowCreateDialog(false);
      resetForm();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const resetForm = () => {
    setNewDomain('encryption_requirements');
    setNewName('');
    setNewDescription('');
    setNewTarget('org');
    setNewTargetValue('org_001');
    setNewMode('enforce');
  };

  const handleTogglePolicy = (policyId: string) => {
    setPolicies(policies.map(p =>
      p.id === policyId ? { ...p, isActive: !p.isActive, updatedAt: new Date().toISOString() } : p
    ));
  };

  const handleDeletePolicy = async (policyId: string) => {
    try {
      await fetch(`/api/policies?id=${policyId}`, { method: 'DELETE' });
      setPolicies(policies.filter(p => p.id !== policyId));
    } catch {
      // Optimistic removal already applied
    }
  };

  const handlePropagate = (policyId: string) => {
    setPropagatingPolicies(prev => new Set(prev).add(policyId));
    setTimeout(() => {
      setPropagatingPolicies(prev => {
        const next = new Set(prev);
        next.delete(policyId);
        return next;
      });
    }, 3000);
  };

  if (loading) {
    return (
      <div>
        <PageHeader title="Policy Engine" description="Configure and manage organizational policies" />
        <div className="flex items-center justify-center py-20">
          <div className="animate-spin h-8 w-8 border-4 border-primary border-t-transparent rounded-full" />
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div>
        <PageHeader title="Policy Engine" description="Configure and manage organizational policies" />
        <div className="rounded-md bg-error-bg border border-error/20 px-4 py-3 mt-4">
          <p className="text-sm text-error">Error: {error}</p>
        </div>
      </div>
    );
  }

  return (
    <div>
      <PageHeader
        title="Policy Engine"
        description="Configure and manage organizational policies across 11 domains with user > role > org precedence"
        actions={
          <Button size="sm" onClick={() => setShowCreateDialog(true)}>
            <Plus size={14} /> Create Policy
          </Button>
        }
      />

      {/* Domain Overview Cards */}
      <div className="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3 xl:grid-cols-4 gap-3 mb-6">
        {(Object.entries(POLICY_DOMAIN_LABELS) as [PolicyDomain, string][]).map(([domain, label]) => {
          const Icon = DOMAIN_ICONS[domain];
          const count = policies.filter(p => p.domain === domain && p.isActive).length;
          return (
            <button
              key={domain}
              onClick={() => setDomainFilter(domainFilter === domain ? '' : domain)}
              className={`flex items-center gap-3 p-3 rounded-lg border text-left transition-colors ${
                domainFilter === domain
                  ? 'border-primary bg-primary/5'
                  : 'border-border hover:bg-background-secondary'
              }`}
            >
              <Icon size={18} className={domainFilter === domain ? 'text-primary' : 'text-foreground-secondary'} />
              <div className="flex-1 min-w-0">
                <p className="text-sm font-medium text-foreground truncate">{label}</p>
                <p className="text-xs text-foreground-tertiary">{count} active</p>
              </div>
            </button>
          );
        })}
      </div>

      {/* Filters */}
      <div className="flex flex-col sm:flex-row gap-3 mb-4">
        <SearchInput
          placeholder="Search policies..."
          onSearch={setSearchQuery}
          className="sm:w-72"
        />
        <Select
          options={[{ value: '', label: 'All Domains' }, ...domainOptions]}
          value={domainFilter}
          onChange={(e) => setDomainFilter(e.target.value)}
          className="sm:w-56"
        />
      </div>

      {/* Policies Table */}
      <Card padding="none">
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Policy</TableHead>
              <TableHead>Domain</TableHead>
              <TableHead className="hidden md:table-cell">Assigned To</TableHead>
              <TableHead>Mode</TableHead>
              <TableHead className="hidden lg:table-cell">Updated</TableHead>
              <TableHead>Status</TableHead>
              <TableHead className="w-32">Actions</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {filteredPolicies.map((policy) => (
              <TableRow key={policy.id}>
                <TableCell>
                  <div>
                    <p className="text-sm font-medium text-foreground">{policy.name}</p>
                    <p className="text-xs text-foreground-tertiary">{policy.description}</p>
                  </div>
                </TableCell>
                <TableCell>
                  <Badge variant="outline">{POLICY_DOMAIN_LABELS[policy.domain]}</Badge>
                </TableCell>
                <TableCell className="hidden md:table-cell">
                  <div className="text-sm">
                    <span className="capitalize">{policy.assignmentTarget}</span>
                    <span className="text-foreground-tertiary ml-1">({policy.assignmentValue})</span>
                  </div>
                </TableCell>
                <TableCell>
                  <Badge variant={
                    policy.enforcementMode === 'enforce' ? 'error' :
                    policy.enforcementMode === 'warn' ? 'warning' :
                    policy.enforcementMode === 'audit_only' ? 'info' : 'default'
                  }>
                    {policy.enforcementMode}
                  </Badge>
                </TableCell>
                <TableCell className="hidden lg:table-cell text-sm text-foreground-secondary">
                  {formatDate(policy.updatedAt)}
                </TableCell>
                <TableCell>
                  <Badge variant={policy.isActive ? 'success' : 'default'}>
                    {policy.isActive ? 'Active' : 'Disabled'}
                  </Badge>
                </TableCell>
                <TableCell>
                  <div className="flex items-center gap-1">
                    <Button
                      variant="ghost"
                      size="sm"
                      onClick={() => { setSelectedPolicy(policy); setShowDetailDialog(true); }}
                    >
                      Edit
                    </Button>
                    <button
                      onClick={() => handlePropagate(policy.id)}
                      className="p-1.5 rounded hover:bg-background-secondary"
                      title="Propagate to clients"
                      disabled={propagatingPolicies.has(policy.id)}
                    >
                      <RefreshCcw size={14} className={`text-foreground-secondary ${propagatingPolicies.has(policy.id) ? 'animate-spin' : ''}`} />
                    </button>
                    <button
                      onClick={() => handleTogglePolicy(policy.id)}
                      className="p-1.5 rounded hover:bg-background-secondary"
                      title={policy.isActive ? 'Disable' : 'Enable'}
                    >
                      <Shield size={14} className={policy.isActive ? 'text-success' : 'text-foreground-disabled'} />
                    </button>
                    <button
                      onClick={() => handleDeletePolicy(policy.id)}
                      className="p-1.5 rounded hover:bg-error-bg"
                      title="Delete"
                    >
                      <Trash2 size={14} className="text-error" />
                    </button>
                  </div>
                </TableCell>
              </TableRow>
            ))}
            {filteredPolicies.length === 0 && (
              <TableRow>
                <TableCell colSpan={7} className="text-center py-8 text-foreground-secondary">
                  No policies found
                </TableCell>
              </TableRow>
            )}
          </TableBody>
        </Table>
      </Card>

      {/* Create Policy Dialog */}
      <Dialog open={showCreateDialog} onClose={() => setShowCreateDialog(false)} className="max-w-xl">
        <DialogHeader>
          <DialogTitle>Create Policy</DialogTitle>
          <DialogDescription>Define a new policy rule for your organization</DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          <Select
            label="Policy Domain"
            options={domainOptions}
            value={newDomain}
            onChange={(e) => setNewDomain(e.target.value as PolicyDomain)}
          />
          <p className="text-xs text-foreground-tertiary -mt-2">
            {POLICY_DOMAIN_DESCRIPTIONS[newDomain]}
          </p>
          <Input
            label="Policy Name"
            placeholder="e.g., Encryption Policy for Legal Team"
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
          />
          <Input
            label="Description"
            placeholder="Describe what this policy enforces"
            value={newDescription}
            onChange={(e) => setNewDescription(e.target.value)}
          />
          <Select
            label="Assign To"
            options={[
              { value: 'org', label: 'Organization (lowest priority)' },
              { value: 'connection_type', label: 'Connection Type' },
              { value: 'role', label: 'Role' },
              { value: 'user', label: 'User (highest priority)' },
            ]}
            value={newTarget}
            onChange={(e) => setNewTarget(e.target.value as PolicyAssignmentTarget)}
          />
          <Input
            label="Assignment Value"
            placeholder={newTarget === 'org' ? 'org_001' : newTarget === 'role' ? 'Role name' : 'User ID or type'}
            value={newTargetValue}
            onChange={(e) => setNewTargetValue(e.target.value)}
            hint="Precedence: user > role > connection_type > org"
          />
          <Select
            label="Enforcement Mode"
            options={[
              { value: 'enforce', label: 'Enforce (block violations)' },
              { value: 'warn', label: 'Warn (allow with warning)' },
              { value: 'audit_only', label: 'Audit Only (log only)' },
              { value: 'disabled', label: 'Disabled' },
            ]}
            value={newMode}
            onChange={(e) => setNewMode(e.target.value as PolicyEnforcementMode)}
          />
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => { setShowCreateDialog(false); resetForm(); }}>Cancel</Button>
          <Button onClick={handleCreatePolicy}>Create Policy</Button>
        </DialogFooter>
      </Dialog>

      {/* Policy Detail Dialog */}
      <Dialog open={showDetailDialog} onClose={() => { setShowDetailDialog(false); setSelectedPolicy(null); }}>
        <DialogHeader>
          <DialogTitle>{selectedPolicy?.name}</DialogTitle>
          <DialogDescription>{selectedPolicy?.description}</DialogDescription>
        </DialogHeader>
        {selectedPolicy && (
          <div className="space-y-3">
            <div className="grid grid-cols-2 gap-4 text-sm">
              <div>
                <p className="text-foreground-tertiary">Domain</p>
                <p className="font-medium">{POLICY_DOMAIN_LABELS[selectedPolicy.domain]}</p>
              </div>
              <div>
                <p className="text-foreground-tertiary">Enforcement</p>
                <Badge variant={
                  selectedPolicy.enforcementMode === 'enforce' ? 'error' :
                  selectedPolicy.enforcementMode === 'warn' ? 'warning' : 'info'
                }>
                  {selectedPolicy.enforcementMode}
                </Badge>
              </div>
              <div>
                <p className="text-foreground-tertiary">Assignment</p>
                <p className="font-medium capitalize">{selectedPolicy.assignmentTarget}: {selectedPolicy.assignmentValue}</p>
              </div>
              <div>
                <p className="text-foreground-tertiary">Version</p>
                <p className="font-medium">v{selectedPolicy.version}</p>
              </div>
            </div>
            <div>
              <p className="text-sm text-foreground-tertiary mb-1">Configuration</p>
              <pre className="text-xs bg-background-secondary rounded-md p-3 overflow-auto max-h-48 font-mono">
                {JSON.stringify(selectedPolicy.configuration, null, 2)}
              </pre>
            </div>
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => { setShowDetailDialog(false); setSelectedPolicy(null); }}>Close</Button>
        </DialogFooter>
      </Dialog>
    </div>
  );
}
