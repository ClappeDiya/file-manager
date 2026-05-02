'use client';

import React, { useState, useEffect } from 'react';
import {
  FolderSync, Plus, Users, Plug, RefreshCcw, Play, Clock,
  ArrowRight, Share2,
} from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { Tabs, TabContent } from '@/components/ui/tabs';
import { Dialog, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from '@/components/ui/dialog';
import { EmptyState } from '@/components/ui/empty-state';
import { StatCard } from '@/components/ui/stat-card';
import { Table, TableHeader, TableBody, TableRow, TableHead, TableCell } from '@/components/ui/table';
import { formatDate } from '@/lib/utils/format';
import type { SharedConnection, SyncTemplate, PropagationStatus, Workspace } from '@/lib/types/workspaces';
import { useToastStore } from '@/lib/stores/toast-store';

function PropagationBadge({ status }: { status: PropagationStatus }) {
  const variant = status.status === 'complete' ? 'success' :
    status.status === 'partial' ? 'warning' :
    status.status === 'failed' ? 'error' : 'info';
  return (
    <Badge variant={variant}>
      {status.confirmedCount}/{status.targetCount} synced
    </Badge>
  );
}

export default function WorkspacesPage() {
  const [sharedConnections, setSharedConnections] = useState<SharedConnection[]>([]);
  const [syncTemplates, setSyncTemplates] = useState<SyncTemplate[]>([]);
  const [workspaces, setWorkspaces] = useState<Workspace[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState('connections');
  const [showShareDialog, setShowShareDialog] = useState(false);
  const [propagating, setPropagating] = useState<Set<string>>(new Set());
  const notify = useToastStore((s) => s.notify);

  useEffect(() => {
    fetch('/api/workspaces')
      .then((res) => {
        if (!res.ok) throw new Error('Failed to fetch workspaces');
        return res.json();
      })
      .then((data) => {
        setSharedConnections(data.connections || []);
        setSyncTemplates(data.syncTemplates || []);
        setWorkspaces(data.workspaces || []);
        setLoading(false);
      })
      .catch((err) => { setError(err instanceof Error ? err.message : String(err)); setLoading(false); });
  }, []);

  const handlePropagate = async (id: string) => {
    setPropagating(prev => new Set(prev).add(id));
    try {
      const response = await fetch('/api/workspaces', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action: 'propagate', resourceId: id }),
      });
      if (!response.ok) throw new Error(`Request failed (${response.status})`);
      notify('Propagation started', 'success');
    } catch (err) {
      notify(`Propagation failed. ${err instanceof Error ? err.message : ''}`.trim(), 'error');
    }
    setTimeout(() => {
      setPropagating(prev => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    }, 3000);
  };

  if (loading) {
    return (
      <div>
        <PageHeader title="Shared Workspaces" description="Manage shared connections and sync templates" />
        <div className="flex items-center justify-center py-20">
          <div className="animate-spin h-8 w-8 border-4 border-primary border-t-transparent rounded-full" />
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div>
        <PageHeader title="Shared Workspaces" description="Manage shared connections and sync templates" />
        <div className="rounded-md bg-error-bg border border-error/20 px-4 py-3 mt-4">
          <p className="text-sm text-error">Error: {error}</p>
        </div>
      </div>
    );
  }

  return (
    <div>
      <PageHeader
        title="Shared Workspaces"
        description="Manage shared connections, sync templates, and automation templates for your team"
        actions={
          <Button size="sm" onClick={() => setShowShareDialog(true)}>
            <Share2 size={14} /> Share Resource
          </Button>
        }
      />

      {/* Stats */}
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-4 mb-6">
        <StatCard
          title="Shared Connections"
          value={sharedConnections.length}
          icon={Plug}
          variant="info"
        />
        <StatCard
          title="Sync Templates"
          value={syncTemplates.length}
          icon={RefreshCcw}
          variant="success"
        />
        <StatCard
          title="Team Members"
          value={workspaces[0]?.members.length || 0}
          icon={Users}
          variant="default"
        />
      </div>

      <Tabs
        tabs={[
          { id: 'connections', label: 'Shared Connections', count: sharedConnections.length },
          { id: 'sync', label: 'Sync Templates', count: syncTemplates.length },
          { id: 'automation', label: 'Automation Templates', count: 0 },
          { id: 'members', label: 'Members' },
        ]}
        activeTab={activeTab}
        onTabChange={setActiveTab}
      >
        {/* Shared Connections */}
        <TabContent tabId="connections" activeTab={activeTab}>
          <div className="space-y-4">
            {sharedConnections.length > 0 ? sharedConnections.map((conn) => (
              <Card key={conn.id}>
                <div className="flex flex-col md:flex-row md:items-center gap-4">
                  <div className="flex items-center gap-3 flex-1 min-w-0">
                    <div className="p-2.5 rounded-lg bg-background-tertiary">
                      <Plug size={18} className="text-foreground-secondary" />
                    </div>
                    <div className="flex-1 min-w-0">
                      <div className="flex items-center gap-2">
                        <h3 className="text-sm font-semibold text-foreground">{conn.name}</h3>
                        <Badge variant={conn.isActive ? 'success' : 'default'}>
                          {conn.isActive ? 'Active' : 'Inactive'}
                        </Badge>
                      </div>
                      <p className="text-xs text-foreground-tertiary">
                        {conn.connectorType.toUpperCase()} - Owned by {conn.ownerName}
                      </p>
                      <p className="text-xs text-foreground-secondary mt-1">
                        Shared with {conn.sharedWith.length} members
                      </p>
                    </div>
                  </div>
                  <div className="flex items-center gap-2">
                    <PropagationBadge status={conn.propagationStatus} />
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => handlePropagate(conn.id)}
                      loading={propagating.has(conn.id)}
                    >
                      <RefreshCcw size={14} /> Propagate
                    </Button>
                  </div>
                </div>
                {/* Shared members */}
                <div className="mt-3 pt-3 border-t border-border">
                  <div className="flex flex-wrap gap-2">
                    {conn.sharedWith.map((member) => (
                      <div key={member.userId} className="flex items-center gap-1.5 text-xs bg-background-secondary rounded-full px-2.5 py-1">
                        <span className="text-foreground">{member.userName}</span>
                        <Badge variant="outline" className="text-xs px-1 py-0">{member.accessLevel}</Badge>
                      </div>
                    ))}
                  </div>
                </div>
              </Card>
            )) : (
              <EmptyState
                icon={Plug}
                title="No shared connections"
                description="Share connections with your team to get started."
              />
            )}
          </div>
        </TabContent>

        {/* Sync Templates */}
        <TabContent tabId="sync" activeTab={activeTab}>
          <div className="space-y-4">
            {syncTemplates.length > 0 ? syncTemplates.map((template) => (
              <Card key={template.id}>
                <div className="flex flex-col md:flex-row md:items-start gap-4">
                  <div className="flex-1 min-w-0">
                    <div className="flex items-center gap-2 mb-1">
                      <h3 className="text-sm font-semibold text-foreground">{template.name}</h3>
                      <Badge variant={template.isActive ? 'success' : 'default'}>
                        {template.isActive ? 'Active' : 'Inactive'}
                      </Badge>
                      <Badge variant="outline">{template.syncMode}</Badge>
                    </div>
                    <p className="text-xs text-foreground-secondary">{template.description}</p>

                    <div className="flex items-center gap-1 text-xs text-foreground-tertiary mt-2">
                      <span>{template.sourceConnector}:{template.sourcePath}</span>
                      <ArrowRight size={12} />
                      <span>{template.destConnector}:{template.destPath}</span>
                    </div>

                    {template.schedule && (
                      <div className="flex items-center gap-1 text-xs text-foreground-tertiary mt-1">
                        <Clock size={12} />
                        <span>Schedule: {template.schedule}</span>
                      </div>
                    )}

                    <p className="text-xs text-foreground-secondary mt-2">
                      Activated by {template.activatedByUsers.length} users - Shared with {template.sharedWith.length} members
                    </p>
                  </div>
                  <div className="flex items-center gap-2">
                    <PropagationBadge status={template.propagationStatus} />
                    <Button variant="outline" size="sm" onClick={() => handlePropagate(template.id)} loading={propagating.has(template.id)}>
                      <RefreshCcw size={14} /> Propagate
                    </Button>
                    <Button variant="primary" size="sm">
                      <Play size={14} /> Activate
                    </Button>
                  </div>
                </div>
              </Card>
            )) : (
              <EmptyState
                icon={RefreshCcw}
                title="No sync templates"
                description="Create sync templates to share repeatable sync workflows."
              />
            )}
          </div>
        </TabContent>

        {/* Automation Templates */}
        <TabContent tabId="automation" activeTab={activeTab}>
          <EmptyState
            icon={FolderSync}
            title="No automation templates yet"
            description="Create automation templates to share repeatable workflows with your team."
            action={<Button><Plus size={14} /> Create Template</Button>}
          />
        </TabContent>

        {/* Members */}
        <TabContent tabId="members" activeTab={activeTab}>
          {workspaces[0] ? (
            <Card padding="none">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>Member</TableHead>
                    <TableHead>Access Level</TableHead>
                    <TableHead>Added</TableHead>
                    <TableHead>Added By</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {workspaces[0].members.map((member) => (
                    <TableRow key={member.userId}>
                      <TableCell>
                        <div>
                          <p className="text-sm font-medium">{member.userName}</p>
                          <p className="text-xs text-foreground-tertiary">{member.userEmail}</p>
                        </div>
                      </TableCell>
                      <TableCell>
                        <Badge variant={
                          member.accessLevel === 'admin' ? 'info' :
                          member.accessLevel === 'write' ? 'success' : 'default'
                        }>
                          {member.accessLevel}
                        </Badge>
                      </TableCell>
                      <TableCell className="text-sm text-foreground-secondary">
                        {formatDate(member.addedAt)}
                      </TableCell>
                      <TableCell className="text-sm text-foreground-secondary">
                        {member.addedBy}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </Card>
          ) : (
            <EmptyState
              icon={Users}
              title="No workspace members"
              description="Create a workspace and add team members."
            />
          )}
        </TabContent>
      </Tabs>

      {/* Share Dialog */}
      <Dialog open={showShareDialog} onClose={() => setShowShareDialog(false)}>
        <DialogHeader>
          <DialogTitle>Share Resource</DialogTitle>
          <DialogDescription>Share a connection, sync template, or automation with team members</DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          <Select
            label="Resource Type"
            options={[
              { value: 'connection', label: 'Connection' },
              { value: 'sync_template', label: 'Sync Template' },
              { value: 'automation', label: 'Automation Template' },
            ]}
          />
          <Input label="User Email" placeholder="user@example.com" />
          <Select
            label="Access Level"
            options={[
              { value: 'read', label: 'Read Only' },
              { value: 'write', label: 'Read & Write' },
              { value: 'admin', label: 'Admin' },
            ]}
          />
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => setShowShareDialog(false)}>Cancel</Button>
          <Button onClick={() => setShowShareDialog(false)}>
            <Share2 size={14} /> Share
          </Button>
        </DialogFooter>
      </Dialog>
    </div>
  );
}
