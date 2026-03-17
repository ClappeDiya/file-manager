'use client';

import React, { useState, useRef, useEffect } from 'react';
import {
  UserPlus, Upload, Download, Shield,
  Mail, UserX, Trash2, Key,
} from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { Card, CardContent } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { Table, TableHeader, TableBody, TableRow, TableHead, TableCell } from '@/components/ui/table';
import { Tabs, TabContent } from '@/components/ui/tabs';
import { Dialog, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from '@/components/ui/dialog';
import { SearchInput } from '@/components/ui/search-input';
import { StatCard } from '@/components/ui/stat-card';
import { ROLE_LABELS, ROLE_PERMISSIONS, type Role, type AdminUser } from '@/lib/types/auth';
import { formatDate, formatRelativeTime } from '@/lib/utils/format';
import { useAuthStore } from '@/lib/stores/auth-store';

export default function UsersPage() {
  const [searchQuery, setSearchQuery] = useState('');
  const [roleFilter, setRoleFilter] = useState('');
  const [activeTab, setActiveTab] = useState('users');
  const [showInviteDialog, setShowInviteDialog] = useState(false);
  const [showRoleDialog, setShowRoleDialog] = useState(false);
  const [selectedUser, setSelectedUser] = useState<AdminUser | null>(null);
  const [inviteEmail, setInviteEmail] = useState('');
  const [inviteRole, setInviteRole] = useState<Role>('user');
  const [users, setUsers] = useState<AdminUser[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const _currentUser = useAuthStore((s) => s.user);

  useEffect(() => {
    fetch('/api/users')
      .then((res) => {
        if (!res.ok) throw new Error('Failed to fetch users');
        return res.json();
      })
      .then((data) => { setUsers(data.users || []); setLoading(false); })
      .catch((err) => { setError(err instanceof Error ? err.message : String(err)); setLoading(false); });
  }, []);

  const filteredUsers = users.filter((user) => {
    const matchesSearch = !searchQuery ||
      user.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
      user.email.toLowerCase().includes(searchQuery.toLowerCase());
    const matchesRole = !roleFilter || user.role === roleFilter;
    return matchesSearch && matchesRole;
  });

  const roleOptions = Object.entries(ROLE_LABELS).map(([value, label]) => ({ value, label }));

  const handleInvite = async () => {
    if (!inviteEmail) return;
    try {
      const res = await fetch('/api/users', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ email: inviteEmail, role: inviteRole, name: inviteEmail.split('@')[0].replace(/[._]/g, ' ').replace(/\b\w/g, c => c.toUpperCase()) }),
      });
      if (!res.ok) throw new Error('Failed to invite user');
      const data = await res.json();
      setUsers([...users, data.user]);
      setShowInviteDialog(false);
      setInviteEmail('');
      setInviteRole('user');
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  const handleRoleChange = (userId: string, newRole: Role) => {
    setUsers(users.map(u => u.id === userId ? { ...u, role: newRole } : u));
    setShowRoleDialog(false);
    setSelectedUser(null);
  };

  const handleDeactivate = async (userId: string) => {
    const user = users.find(u => u.id === userId);
    if (!user) return;
    const action = user.isActive ? 'deactivate' : 'activate';
    try {
      await fetch('/api/users', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action, userIds: [userId] }),
      });
      setUsers(users.map(u => u.id === userId ? { ...u, isActive: !u.isActive } : u));
    } catch {
      // Optimistic update already applied
    }
  };

  const handleRemove = async (userId: string) => {
    try {
      await fetch('/api/users', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action: 'remove', userIds: [userId] }),
      });
      setUsers(users.filter(u => u.id !== userId));
    } catch {
      // Optimistic update already applied
    }
  };

  const handleCSVImport = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;

    const reader = new FileReader();
    reader.onload = (event) => {
      const text = event.target?.result as string;
      const lines = text.split('\n').slice(1); // skip header
      const imported: AdminUser[] = lines
        .filter(line => line.trim())
        .map((line, i) => {
          const [email, name, role] = line.split(',').map(s => s.trim());
          return {
            id: `usr_import_${Date.now()}_${i}`,
            email: email || '',
            name: name || email?.split('@')[0] || '',
            role: (role as Role) || 'user',
            permissions: [],
            organizationId: 'org_001',
            createdAt: new Date().toISOString(),
            isActive: true,
          };
        });
      setUsers([...users, ...imported]);
    };
    reader.readAsText(file);
    e.target.value = '';
  };

  const handleCSVExport = () => {
    const header = 'Email,Name,Role,Status,Created At,Last Login';
    const rows = users.map(u =>
      `${u.email},${u.name},${u.role},${u.isActive ? 'active' : 'inactive'},${u.createdAt},${u.lastLoginAt || 'never'}`
    );
    const csv = [header, ...rows].join('\n');
    const blob = new Blob([csv], { type: 'text/csv' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'users-export.csv';
    a.click();
    URL.revokeObjectURL(url);
  };

  if (loading) {
    return (
      <div>
        <PageHeader title="Users & Roles" description="Manage user accounts, role assignments, and SSO configuration" />
        <div className="flex items-center justify-center py-20">
          <div className="animate-spin h-8 w-8 border-4 border-primary border-t-transparent rounded-full" />
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div>
        <PageHeader title="Users & Roles" description="Manage user accounts, role assignments, and SSO configuration" />
        <div className="rounded-md bg-error-bg border border-error/20 px-4 py-3 mt-4">
          <p className="text-sm text-error">Error: {error}</p>
        </div>
      </div>
    );
  }

  const activeUsers = users.filter(u => u.isActive).length;
  const ssoUsers = users.filter(u => u.ssoProvider).length;

  return (
    <div>
      <PageHeader
        title="Users & Roles"
        description="Manage user accounts, role assignments, and SSO configuration"
        actions={
          <div className="flex gap-2">
            <input
              ref={fileInputRef}
              type="file"
              accept=".csv"
              onChange={handleCSVImport}
              className="hidden"
            />
            <Button variant="outline" size="sm" onClick={() => fileInputRef.current?.click()}>
              <Upload size={14} /> Import CSV
            </Button>
            <Button variant="outline" size="sm" onClick={handleCSVExport}>
              <Download size={14} /> Export CSV
            </Button>
            <Button size="sm" onClick={() => setShowInviteDialog(true)}>
              <UserPlus size={14} /> Invite User
            </Button>
          </div>
        }
      />

      {/* Stats */}
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-4 mb-6">
        <StatCard title="Total Users" value={users.length} icon={Shield} variant="info" />
        <StatCard title="Active Users" value={activeUsers} icon={Shield} variant="success" />
        <StatCard title="SSO Users" value={ssoUsers} icon={Key} variant="default" subtitle="SAML/SCIM" />
      </div>

      <Tabs
        tabs={[
          { id: 'users', label: 'Users', count: users.length },
          { id: 'roles', label: 'Roles', count: 7 },
          { id: 'sso', label: 'SSO Configuration' },
        ]}
        activeTab={activeTab}
        onTabChange={setActiveTab}
      >
        <TabContent tabId="users" activeTab={activeTab}>
          {/* Filters */}
          <div className="flex flex-col sm:flex-row gap-3 mb-4">
            <SearchInput
              placeholder="Search by name or email..."
              onSearch={setSearchQuery}
              className="sm:w-72"
            />
            <Select
              options={[{ value: '', label: 'All Roles' }, ...roleOptions]}
              value={roleFilter}
              onChange={(e) => setRoleFilter(e.target.value)}
              className="sm:w-48"
            />
          </div>

          <Card padding="none">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>User</TableHead>
                  <TableHead>Role</TableHead>
                  <TableHead className="hidden md:table-cell">Status</TableHead>
                  <TableHead className="hidden lg:table-cell">Last Login</TableHead>
                  <TableHead className="hidden lg:table-cell">Created</TableHead>
                  <TableHead className="w-12">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {filteredUsers.map((user) => (
                  <TableRow key={user.id}>
                    <TableCell>
                      <div className="flex items-center gap-3">
                        <div className="w-8 h-8 rounded-full bg-primary/10 flex items-center justify-center shrink-0">
                          <span className="text-xs font-medium text-primary">
                            {user.name.split(' ').map(n => n[0]).join('').toUpperCase()}
                          </span>
                        </div>
                        <div>
                          <p className="text-sm font-medium text-foreground">{user.name}</p>
                          <p className="text-xs text-foreground-tertiary">{user.email}</p>
                        </div>
                      </div>
                    </TableCell>
                    <TableCell>
                      <Badge variant="outline">{ROLE_LABELS[user.role]}</Badge>
                    </TableCell>
                    <TableCell className="hidden md:table-cell">
                      <Badge variant={user.isActive ? 'success' : 'default'}>
                        {user.isActive ? 'Active' : 'Inactive'}
                      </Badge>
                    </TableCell>
                    <TableCell className="hidden lg:table-cell text-sm text-foreground-secondary">
                      {user.lastLoginAt ? formatRelativeTime(user.lastLoginAt) : 'Never'}
                    </TableCell>
                    <TableCell className="hidden lg:table-cell text-sm text-foreground-secondary">
                      {formatDate(user.createdAt)}
                    </TableCell>
                    <TableCell>
                      <div className="flex items-center gap-1">
                        <button
                          onClick={() => { setSelectedUser(user); setShowRoleDialog(true); }}
                          className="p-1.5 rounded hover:bg-background-secondary"
                          title="Change role"
                        >
                          <Shield size={14} className="text-foreground-secondary" />
                        </button>
                        <button
                          onClick={() => handleDeactivate(user.id)}
                          className="p-1.5 rounded hover:bg-background-secondary"
                          title={user.isActive ? 'Deactivate' : 'Activate'}
                        >
                          <UserX size={14} className="text-foreground-secondary" />
                        </button>
                        <button
                          onClick={() => handleRemove(user.id)}
                          className="p-1.5 rounded hover:bg-error-bg"
                          title="Remove"
                        >
                          <Trash2 size={14} className="text-error" />
                        </button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
                {filteredUsers.length === 0 && (
                  <TableRow>
                    <TableCell colSpan={6} className="text-center py-8 text-foreground-secondary">
                      No users found
                    </TableCell>
                  </TableRow>
                )}
              </TableBody>
            </Table>
          </Card>
        </TabContent>

        <TabContent tabId="roles" activeTab={activeTab}>
          <div className="grid gap-4">
            {(Object.entries(ROLE_LABELS) as [Role, string][]).map(([role, label]) => (
              <Card key={role}>
                <div className="flex items-start justify-between">
                  <div>
                    <h3 className="text-sm font-semibold text-foreground">{label}</h3>
                    <p className="text-xs text-foreground-tertiary mt-1">
                      {ROLE_PERMISSIONS[role].length} permissions
                    </p>
                  </div>
                  <Badge variant="outline">
                    {users.filter(u => u.role === role).length} users
                  </Badge>
                </div>
                <div className="flex flex-wrap gap-1 mt-3">
                  {ROLE_PERMISSIONS[role].map((perm) => (
                    <Badge key={perm} variant="default" className="text-xs">
                      {perm}
                    </Badge>
                  ))}
                  {ROLE_PERMISSIONS[role].length === 0 && (
                    <span className="text-xs text-foreground-tertiary">No admin permissions (standard user)</span>
                  )}
                </div>
              </Card>
            ))}
          </div>
        </TabContent>

        <TabContent tabId="sso" activeTab={activeTab}>
          <div className="grid gap-6 max-w-2xl">
            <Card>
              <CardContent>
                <h3 className="text-lg font-semibold text-foreground mb-4">SAML 2.0 Configuration (Business)</h3>
                <div className="space-y-4">
                  <Input label="Entity ID / Issuer" placeholder="https://idp.example.com/saml/metadata" />
                  <Input label="SSO URL" placeholder="https://idp.example.com/saml/sso" />
                  <div className="flex flex-col gap-1.5">
                    <label className="text-sm font-medium text-foreground">X.509 Certificate</label>
                    <textarea
                      className="w-full h-32 rounded-md border border-border bg-background px-3 py-2 text-sm font-mono placeholder:text-foreground-disabled focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary"
                      placeholder="Paste your IdP certificate here..."
                    />
                  </div>
                  <div className="flex items-center gap-2">
                    <input type="checkbox" id="saml-enabled" className="rounded" />
                    <label htmlFor="saml-enabled" className="text-sm text-foreground">Enable SAML SSO</label>
                  </div>
                  <Button>Save SAML Configuration</Button>
                </div>
              </CardContent>
            </Card>

            <Card>
              <CardContent>
                <h3 className="text-lg font-semibold text-foreground mb-4">SCIM Provisioning (Enterprise)</h3>
                <div className="space-y-4">
                  <Input
                    label="SCIM Endpoint"
                    value="https://api.ufop.io/scim/v2/org_001"
                    readOnly
                    hint="Provide this URL to your identity provider"
                  />
                  <Input
                    label="SCIM Bearer Token"
                    type="password"
                    value="scim_token_placeholder"
                    readOnly
                    hint="Use this token for SCIM authentication"
                  />
                  <div className="flex items-center gap-2">
                    <input type="checkbox" id="scim-enabled" className="rounded" />
                    <label htmlFor="scim-enabled" className="text-sm text-foreground">Enable SCIM Provisioning</label>
                  </div>
                  <Button>Regenerate SCIM Token</Button>
                </div>
              </CardContent>
            </Card>
          </div>
        </TabContent>
      </Tabs>

      {/* Invite Dialog */}
      <Dialog open={showInviteDialog} onClose={() => setShowInviteDialog(false)}>
        <DialogHeader>
          <DialogTitle>Invite User</DialogTitle>
          <DialogDescription>Send an invitation to join your organization</DialogDescription>
        </DialogHeader>
        <div className="space-y-4">
          <Input
            label="Email Address"
            type="email"
            placeholder="user@example.com"
            value={inviteEmail}
            onChange={(e) => setInviteEmail(e.target.value)}
          />
          <Select
            label="Role"
            options={roleOptions}
            value={inviteRole}
            onChange={(e) => setInviteRole(e.target.value as Role)}
          />
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => setShowInviteDialog(false)}>Cancel</Button>
          <Button onClick={handleInvite}>
            <Mail size={14} /> Send Invitation
          </Button>
        </DialogFooter>
      </Dialog>

      {/* Role Change Dialog */}
      <Dialog open={showRoleDialog} onClose={() => { setShowRoleDialog(false); setSelectedUser(null); }}>
        <DialogHeader>
          <DialogTitle>Change Role</DialogTitle>
          <DialogDescription>
            {selectedUser && `Update role for ${selectedUser.name} (${selectedUser.email})`}
          </DialogDescription>
        </DialogHeader>
        {selectedUser && (
          <div className="space-y-4">
            <p className="text-sm text-foreground-secondary">
              Current role: <Badge variant="outline">{ROLE_LABELS[selectedUser.role]}</Badge>
            </p>
            <Select
              label="New Role"
              options={roleOptions}
              value={selectedUser.role}
              onChange={(e) => handleRoleChange(selectedUser.id, e.target.value as Role)}
            />
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => { setShowRoleDialog(false); setSelectedUser(null); }}>Cancel</Button>
        </DialogFooter>
      </Dialog>
    </div>
  );
}
