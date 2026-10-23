'use client';

/** Users → SSO & provisioning: SCIM tokens for the identity provider. */
import React, { useState } from 'react';
import { Copy } from 'lucide-react';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table';
import { ErrorBanner, StatusBadge } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { formatRelativeTime } from '@/lib/utils/format';
import type { ScimToken } from '@/lib/db/repos/scim';

export function ScimProvisioning() {
  const { data, error, reload } = useApi<{ tokens: ScimToken[]; baseUrl: string; roles: string[] }>('/api/scim-tokens');
  const [name, setName] = useState('');
  const [role, setRole] = useState('user');
  const [secret, setSecret] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);

  const create = async () => {
    setErr(null);
    try {
      const r = await api<{ secret: string }>('/api/scim-tokens', { body: { name, defaultRole: role } });
      setSecret(r.secret);
      setName('');
      await reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };
  const revoke = async (t: ScimToken) => {
    if (!confirm(`Revoke "${t.name}"? Provisioning with it stops immediately.`)) return;
    try {
      await api(`/api/scim-tokens/${t.id}`, { method: 'DELETE' });
      await reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };

  return (
    <div className="space-y-3 text-sm" data-testid="scim-provisioning">
      <p className="text-foreground-secondary">
        Your identity provider (Entra ID, Okta, …) creates, updates and deactivates users and groups here. Provisioned users get the token&apos;s default role; administrators and groups
        created in the console are never changed by SCIM.
      </p>
      <ErrorBanner message={err ?? error} />
      {data && (
        <div>
          <p className="text-xs font-medium text-foreground-secondary">Tenant URL</p>
          <div className="flex items-center gap-2">
            <code className="flex-1 break-all rounded bg-background-secondary p-2 text-xs">{data.baseUrl}</code>
            <Button size="icon" variant="ghost" aria-label="Copy tenant URL" onClick={() => void navigator.clipboard.writeText(data.baseUrl)}>
              <Copy size={14} />
            </Button>
          </div>
        </div>
      )}
      {secret && (
        <div className="rounded border border-amber-300 p-2">
          <p className="text-xs font-medium">Secret token — copy it now, it is not shown again</p>
          <code className="block break-all text-xs">{secret}</code>
          <Button size="sm" variant="outline" className="mt-1" onClick={() => setSecret(null)}>
            Done
          </Button>
        </div>
      )}
      <div className="flex flex-wrap items-end gap-2">
        <Input label="Token name" placeholder="Entra ID" value={name} onChange={(e) => setName(e.target.value)} />
        <Select label="Default role" value={role} onChange={(e) => setRole(e.target.value)} options={(data?.roles ?? ['user']).map((r) => ({ value: r, label: r.replace(/_/g, ' ') }))} />
        <Button disabled={!name.trim()} onClick={() => void create()}>
          Create token
        </Button>
      </div>
      {!!data?.tokens.length && (
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Token</TableHead>
              <TableHead>Default role</TableHead>
              <TableHead>Last used</TableHead>
              <TableHead />
            </TableRow>
          </TableHeader>
          <TableBody>
            {data.tokens.map((t) => (
              <TableRow key={t.id}>
                <TableCell>
                  {t.name} <span className="font-mono text-xs text-foreground-tertiary">{t.tokenPrefix}…</span>
                  {t.revokedAt && <StatusBadge value="revoked" />}
                </TableCell>
                <TableCell className="text-xs">{t.defaultRole.replace(/_/g, ' ')}</TableCell>
                <TableCell className="text-xs">{t.lastUsedAt ? formatRelativeTime(t.lastUsedAt) : 'never'}</TableCell>
                <TableCell className="text-right">
                  {!t.revokedAt && (
                    <Button size="sm" variant="ghost" onClick={() => void revoke(t)}>
                      Revoke
                    </Button>
                  )}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      )}
    </div>
  );
}
