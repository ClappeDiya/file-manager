'use client';

import React, { useState } from 'react';
import { Plug, Plus, Settings, Trash2, TestTube, CheckCircle, XCircle } from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card, CardContent } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { Dialog, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from '@/components/ui/dialog';
import { EmptyState } from '@/components/ui/empty-state';

interface Connector {
  id: string;
  name: string;
  type: string;
  status: 'connected' | 'disconnected' | 'error';
  lastTestedAt?: string;
  config: Record<string, string>;
}

const CONNECTOR_TYPES = [
  { value: 's3', label: 'Amazon S3' },
  { value: 'azure_blob', label: 'Azure Blob Storage' },
  { value: 'gcs', label: 'Google Cloud Storage' },
  { value: 'sftp', label: 'SFTP' },
  { value: 'ftp', label: 'FTP' },
  { value: 'webdav', label: 'WebDAV' },
  { value: 'smb', label: 'SMB/CIFS' },
  { value: 'local', label: 'Local File System' },
];

export default function ConnectorsPage() {
  const [connectors, setConnectors] = useState<Connector[]>([
    { id: 'conn_001', name: 'Production S3', type: 's3', status: 'connected', lastTestedAt: '2026-03-15T08:00:00Z', config: { bucket: 'acme-prod', region: 'us-east-1' } },
    { id: 'conn_002', name: 'Archive Azure', type: 'azure_blob', status: 'connected', lastTestedAt: '2026-03-15T08:00:00Z', config: { container: 'archive', account: 'acmestorage' } },
    { id: 'conn_003', name: 'Partner SFTP', type: 'sftp', status: 'disconnected', config: { host: 'sftp.partner.com', port: '22' } },
  ]);
  const [showAddDialog, setShowAddDialog] = useState(false);
  const [newName, setNewName] = useState('');
  const [newType, setNewType] = useState('s3');
  const [testingId, setTestingId] = useState<string | null>(null);

  const handleAdd = () => {
    if (!newName) return;
    setConnectors([...connectors, {
      id: `conn_${Date.now()}`,
      name: newName,
      type: newType,
      status: 'disconnected',
      config: {},
    }]);
    setShowAddDialog(false);
    setNewName('');
    setNewType('s3');
  };

  const handleTest = (id: string) => {
    setTestingId(id);
    setTimeout(() => {
      setConnectors(prev => prev.map(c =>
        c.id === id ? { ...c, status: 'connected' as const, lastTestedAt: new Date().toISOString() } : c
      ));
      setTestingId(null);
    }, 2000);
  };

  const handleRemove = (id: string) => {
    setConnectors(prev => prev.filter(c => c.id !== id));
  };

  return (
    <div>
      <PageHeader
        title="Connectors"
        description="Manage storage connectors and integrations"
        actions={
          <Button size="sm" onClick={() => setShowAddDialog(true)}>
            <Plus size={14} /> Add Connector
          </Button>
        }
      />

      {connectors.length === 0 ? (
        <EmptyState
          icon={Plug}
          title="No connectors configured"
          description="Add a storage connector to start managing file operations across platforms."
          action={<Button onClick={() => setShowAddDialog(true)}><Plus size={14} /> Add Connector</Button>}
        />
      ) : (
        <div className="grid gap-4">
          {connectors.map((connector) => (
            <Card key={connector.id}>
              <div className="flex items-center gap-4">
                <div className="p-3 rounded-lg bg-background-tertiary">
                  <Plug size={20} className="text-foreground-secondary" />
                </div>
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2">
                    <h3 className="text-sm font-semibold text-foreground">{connector.name}</h3>
                    <Badge variant={
                      connector.status === 'connected' ? 'success' :
                      connector.status === 'error' ? 'error' : 'default'
                    }>
                      {connector.status}
                    </Badge>
                  </div>
                  <p className="text-xs text-foreground-tertiary mt-0.5">
                    {CONNECTOR_TYPES.find(t => t.value === connector.type)?.label || connector.type}
                    {connector.lastTestedAt && ` - Last tested: ${new Date(connector.lastTestedAt).toLocaleString()}`}
                  </p>
                </div>
                <div className="flex items-center gap-1">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => handleTest(connector.id)}
                    loading={testingId === connector.id}
                  >
                    <TestTube size={14} /> Test
                  </Button>
                  <button className="p-2 rounded hover:bg-background-secondary" title="Settings">
                    <Settings size={14} className="text-foreground-secondary" />
                  </button>
                  <button className="p-2 rounded hover:bg-error-bg" title="Remove" onClick={() => handleRemove(connector.id)}>
                    <Trash2 size={14} className="text-error" />
                  </button>
                </div>
              </div>
            </Card>
          ))}
        </div>
      )}

      <Dialog open={showAddDialog} onClose={() => setShowAddDialog(false)}>
        <DialogHeader>
          <DialogTitle>Add Connector</DialogTitle>
          <DialogDescription>Configure a new storage connector</DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          <Input label="Name" placeholder="e.g., Production S3" value={newName} onChange={(e) => setNewName(e.target.value)} />
          <Select label="Type" options={CONNECTOR_TYPES} value={newType} onChange={(e) => setNewType(e.target.value)} />
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => setShowAddDialog(false)}>Cancel</Button>
          <Button onClick={handleAdd}>Add Connector</Button>
        </DialogFooter>
      </Dialog>
    </div>
  );
}
