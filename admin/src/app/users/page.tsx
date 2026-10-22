'use client';

import React, { useState, useRef, useEffect } from 'react';
import {
  UserPlus, Upload, Download, Shield,
  Mail, UserX, Trash2, Key, CalendarClock, HelpCircle,
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
import type { Invitation } from '@/lib/db/repos/invitations';
import { useToastStore } from '@/lib/stores/toast-store';
import { z } from 'zod';
import Link from 'next/link';
import { ComingSoon } from '@/components/ui/coming-soon';
import { ScimProvisioning } from '@/components/users/scim-provisioning';
import { CsvImportDialog } from '@/components/users/csv-import-dialog';
import { isEnabled } from '@/lib/features';
import { api } from '@/lib/api-client';
import { MembershipExpiryDialog, RemoveMemberDialog, RemovedMembers } from '@/components/users/offboarding';
import { ExplainAccessDialog } from '@/components/users/explain-access';
import type { RemovedMember } from '@/lib/offboarding';

type Member = AdminUser & { expiresAt?: string | null };

const inviteSchema = z.object({
  email: z.string().trim().min(1, 'Email is required').email('Enter a valid email address'),
  role: z.string().min(1, 'Pick a role'),
});

export default function UsersPage() {
  const [searchQuery, setSearchQuery] = useState('');
  const [roleFilter, setRoleFilter] = useState('');
  const [activeTab, setActiveTab] = useState('users');
  const [showInviteDialog, setShowInviteDialog] = useState(false);
  const [showRoleDialog, setShowRoleDialog] = useState(false);
  const [selectedUser, setSelectedUser] = useState<AdminUser | null>(null);
  const [inviteEmail, setInviteEmail] = useState('');
  const [inviteRole, setInviteRole] = useState<Role>('user');
  const [inviteErrors, setInviteErrors] = useState<{ email?: string; role?: string }>({});
  const [users, setUsers] = useState<Member[]>([]);
  const [removed, setRemoved] = useState<RemovedMember[]>([]);
  const [restoreDays, setRestoreDays] = useState(30);
  const [removing, setRemoving] = useState<AdminUser | null>(null);
  const [expiring, setExpiring] = useState<Member | null>(null);
  const [explaining, setExplaining] = useState<AdminUser | null>(null);
  const [invitations, setInvitations] = useState<Invitation[]>([]);
  const [inviteLink, setInviteLink] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [importCsv, setImportCsv] = useState<string | null>(null);
  const _currentUser = useAuthStore((s) => s.user);
  const notify = useToastStore((s) => s.notify);

  useEffect(() => {
    fetch('/api/users')
      .then((res) => {
        if (!res.ok) throw new Error('Failed to fetch users');
        return res.json();
      })
      .then((data) => { setUsers(data.users || []); setInvitations(data.invitations || []); setRemoved(data.removed || []); setRestoreDays(data.restoreDays ?? 30); setLoading(false); })
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
    const parsed = inviteSchema.safeParse({ email: inviteEmail, role: inviteRole });
    if (!parsed.success) {
      const fieldErrors: { email?: string; role?: string } = {};
      for (const issue of parsed.error.issues) {
        const key = issue.path[0] as 'email' | 'role';
        if (!fieldErrors[key]) fieldErrors[key] = issue.message;
      }
      setInviteErrors(fieldErrors);
      return;
    }
    setInviteErrors({});
    try {
      const res = await fetch('/api/users', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          email: parsed.data.email,
          role: parsed.data.role,
          name: parsed.data.email.split('@')[0].replace(/[._]/g, ' ').replace(/\b\w/g, (c) => c.toUpperCase()),
        }),
      });
      const data = await res.json();
      if (!res.ok) throw new Error(data.error || `Failed to invite user (${res.status})`);
      setInvitations([data.invitation, ...invitations]);
      setShowInviteDialog(false);
      setInviteEmail('');
      setInviteRole('user');
      if (data.emailed) notify('Invitation emailed', 'success');
      else setInviteLink(data.link);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setError(message);
      notify(`Could not invite user. ${message}`, 'error');
    }
  };

  const handleRoleChange = async (userId: string, newRole: Role) => {
    try {
      // Through the API client: a role change may ask the admin to confirm it is them.
      await api('/api/users', { method: 'PATCH', body: { action: 'set_role', userIds: [userId], role: newRole } });
      setUsers(users.map(u => u.id === userId ? { ...u, role: newRole } : u));
      notify('Role updated', 'success');
    } catch (err) {
      notify(`Could not change the role. ${err instanceof Error ? err.message : ''}`.trim(), 'error');
    }
    setShowRoleDialog(false);
    setSelectedUser(null);
  };

  const handleRevokeInvitation = async (id: string) => {
    try {
      const response = await fetch(`/api/invitations/${id}`, { method: 'DELETE' });
      if (!response.ok) throw new Error(`Request failed (${response.status})`);
      setInvitations(invitations.map((i) => (i.id === id ? { ...i, status: 'revoked' as const } : i)));
    } catch (err) {
      notify(`Could not revoke. ${err instanceof Error ? err.message : ''}`.trim(), 'error');
    }
  };

  const handleDeactivate = async (userId: string) => {
    const user = users.find(u => u.id === userId);
    if (!user) return;
    const action = user.isActive ? 'deactivate' : 'activate';
    try {
      const response = await fetch('/api/users', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action, userIds: [userId] }),
      });
      if (!response.ok) throw new Error(`Request failed (${response.status})`);
      setUsers(users.map(u => u.id === userId ? { ...u, isActive: !u.isActive } : u));
      notify(`User ${action}d`, 'success');
    } catch (err) {
      notify(`Could not ${action} user. ${err instanceof Error ? err.message : ''}`.trim(), 'error');
    }
  };

  const handleCSVImport = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    e.target.value = '';
    if (!file) return;
    // Parsed and checked on the server (preview first, then invitations).
    void file.text().then(setImportCsv);
  };

  const reloadUsers = () =>
    fetch('/api/users')
      .then((res) => res.json())
      .then((data) => {
        setUsers(data.users || []);
        setInvitations(data.invitations || []);
        setRemoved(data.removed || []);
      })
      .catch(() => undefined);

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
            <Button
              variant="outline"
              size="sm"
              disabled={!isEnabled('csvUserImport')}
              title={isEnabled('csvUserImport') ? undefined : 'CSV import is coming soon. Use Invite User meanwhile.'}
              onClick={() => fileInputRef.current?.click()}
            >
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
                          onClick={() => setExplaining(user)}
                          className="p-1.5 rounded hover:bg-background-secondary"
                          title="Explain access"
                        >
                          <HelpCircle size={14} className="text-foreground-secondary" />
                        </button>
                        <button
                          onClick={() => setExpiring(user)}
                          className="p-1.5 rounded hover:bg-background-secondary"
                          title={user.expiresAt ? `Membership ends ${formatDate(user.expiresAt)}` : 'Set membership end'}
                        >
                          <CalendarClock size={14} className={user.expiresAt ? 'text-warning' : 'text-foreground-secondary'} />
                        </button>
                        <button
                          onClick={() => setRemoving(user)}
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
          {invitations.some((i) => i.status === 'pending') && (
            <Card className="mt-4">
              <h3 className="mb-2 text-sm font-semibold">Pending invitations</h3>
              <ul className="divide-y divide-border">
                {invitations.filter((i) => i.status === 'pending').map((i) => (
                  <li key={i.id} className="flex items-center justify-between py-2 text-sm">
                    <span>
                      {i.email} <span className="text-xs text-foreground-tertiary">as {ROLE_LABELS[i.role]} · invited by {i.invitedBy} · expires {formatRelativeTime(i.expiresAt)}</span>
                    </span>
                    <Button size="sm" variant="ghost" onClick={() => void handleRevokeInvitation(i.id)}>
                      Revoke
                    </Button>
                  </li>
                ))}
              </ul>
            </Card>
          )}
          <RemovedMembers removed={removed} onRestored={() => { void reloadUsers(); notify('Member restored', 'success'); }} />
          <RemoveMemberDialog
            user={removing}
            members={users}
            restoreDays={restoreDays}
            onClose={() => setRemoving(null)}
            onDone={() => { setRemoving(null); void reloadUsers(); notify('User removed', 'success'); }}
          />
          <ExplainAccessDialog user={explaining} onClose={() => setExplaining(null)} />
          <MembershipExpiryDialog user={expiring} onClose={() => setExpiring(null)} onDone={() => { setExpiring(null); void reloadUsers(); notify('Membership end saved', 'success'); }} />
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
                <h3 className="text-lg font-semibold text-foreground mb-2">Single sign-on (OIDC / SAML)</h3>
                <p className="text-sm text-foreground-secondary mb-4">
                  Identity providers, just-in-time accounts, group mapping and MFA requirements are configured per organization.
                </p>
                <Link href="/settings/sso" className="text-sm font-medium text-primary hover:underline">
                  Open SSO settings →
                </Link>
              </CardContent>
            </Card>

            <Card>
              <CardContent>
                <h3 className="text-lg font-semibold text-foreground mb-4">SCIM Provisioning (Enterprise)</h3>
                {isEnabled('scim') ? (
                  <ScimProvisioning />
                ) : (
                  <ComingSoon
                    feature="scim"
                    description="Your identity provider will create, update and deactivate users and groups here automatically. Until then, use just-in-time provisioning in SSO settings."
                  />
                )}
              </CardContent>
            </Card>
          </div>
        </TabContent>
      </Tabs>

      <CsvImportDialog csv={importCsv} onClose={() => setImportCsv(null)} onImported={() => void reloadUsers()} />

      {/* Invite Dialog */}
      <Dialog
        open={showInviteDialog}
        onClose={() => {
          setShowInviteDialog(false);
          setInviteErrors({});
        }}
      >
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
            error={inviteErrors.email}
          />
          <Select
            label="Role"
            options={roleOptions}
            value={inviteRole}
            onChange={(e) => setInviteRole(e.target.value as Role)}
            error={inviteErrors.role}
          />
        </div>
        <DialogFooter>
          <Button
            variant="outline"
            onClick={() => {
              setShowInviteDialog(false);
              setInviteErrors({});
            }}
          >
            Cancel
          </Button>
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
              onChange={(e) => void handleRoleChange(selectedUser.id, e.target.value as Role)}
            />
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => { setShowRoleDialog(false); setSelectedUser(null); }}>Cancel</Button>
        </DialogFooter>
      </Dialog>
      <Dialog open={!!inviteLink} onClose={() => setInviteLink(null)}>
        <DialogHeader>
          <DialogTitle>Invitation created</DialogTitle>
          <DialogDescription>No email channel is set up for invitations. Send this one-time link yourself.</DialogDescription>
        </DialogHeader>
        <code className="block break-all rounded bg-background-secondary p-2 text-xs">{inviteLink}</code>
        <DialogFooter>
          <Button variant="outline" onClick={() => inviteLink && void navigator.clipboard.writeText(inviteLink)}>Copy</Button>
          <Button onClick={() => setInviteLink(null)}>Done</Button>
        </DialogFooter>
      </Dialog>
    </div>
  );
}
