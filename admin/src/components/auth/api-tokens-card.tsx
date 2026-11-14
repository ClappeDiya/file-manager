'use client';

/** My account → API tokens: scoped, expiring tokens for scripts (`Authorization: Bearer ufop_cat_…`). */
import React, { useState } from 'react';
import { Card, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { ErrorBanner } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { formatRelativeTime } from '@/lib/utils/format';
import type { ApiToken } from '@/lib/db/repos/api-tokens';
import type { Permission } from '@/lib/types/auth';

const readOnly = (ps: Permission[]) => ps.filter((p) => p.endsWith('.read') || p === 'admin.access');

export function ApiTokensCard() {
  const { data, reload } = useApi<{
    tokens: ApiToken[];
    grantable: Permission[];
    maxDays: number;
  }>('/api/api-tokens');
  const [err, setErr] = useState<string | null>(null);
  const [name, setName] = useState('');
  const [days, setDays] = useState(90);
  const [preset, setPreset] = useState<'read' | 'all' | 'custom'>('read');
  const [custom, setCustom] = useState<Permission[]>([]);
  const [secret, setSecret] = useState<string | null>(null);
  const grantable = data?.grantable ?? [];
  const chosen = preset === 'read' ? readOnly(grantable) : preset === 'all' ? grantable : custom;

  const run = async (f: () => Promise<void>) => {
    setErr(null);
    try {
      await f();
      await reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };
  const create = () =>
    run(async () => {
      const r = await api<{ secret: string }>('/api/api-tokens', {
        body: { name, permissions: chosen, expiresInDays: days },
      });
      setSecret(r.secret);
      setName('');
    });
  const active = (t: ApiToken) => !t.revokedAt && new Date(t.expiresAt) > new Date();

  return (
    <Card>
      <CardHeader className="flex-col items-start gap-1">
        <CardTitle>API tokens</CardTitle>
        <CardDescription>
          For scripts: send <code>Authorization: Bearer &lt;token&gt;</code>. A token has at most your permissions (and loses any your role loses), expires, and cannot change account or sign-in settings.
        </CardDescription>
      </CardHeader>
      <ErrorBanner message={err} />
      {secret && (
        <div className="mb-3 rounded-md border border-border bg-background-secondary p-2 text-sm">
          Copy the token now; it is not shown again:
          <code className="mt-1 block break-all font-mono text-xs">{secret}</code>
          <Button size="sm" variant="ghost" onClick={() => setSecret(null)}>
            Done
          </Button>
        </div>
      )}
      <ul className="mb-3 space-y-1 text-sm">
        {data?.tokens.map((t) => (
          <li key={t.id} className="flex items-center justify-between gap-2">
            <span>
              {t.name} <span className="font-mono text-xs">{t.prefix}…</span>{' '}
              <span className="text-xs text-foreground-tertiary">
                · {t.permissions.length} permissions · {t.revokedAt ? 'revoked' : new Date(t.expiresAt) <= new Date() ? 'expired' : `expires ${t.expiresAt.slice(0, 10)}`} ·{' '}
                {t.lastUsedAt ? `used ${formatRelativeTime(t.lastUsedAt)}` : 'never used'}
              </span>
            </span>
            {active(t) && (
              <Button size="sm" variant="ghost" onClick={() => void run(() => api(`/api/api-tokens/${t.id}`, { method: 'DELETE' }).then(() => undefined))}>
                Revoke
              </Button>
            )}
          </li>
        ))}
      </ul>
      <div className="space-y-2">
        <Input label="Name" value={name} onChange={(e) => setName(e.target.value)} placeholder="Nightly report script" />
        <Input label={`Expires after (days, 1–${data?.maxDays ?? 365})`} type="number" min={1} max={data?.maxDays ?? 365} value={days} onChange={(e) => setDays(Math.floor(Number(e.target.value) || 1))} />
        <div className="flex flex-wrap gap-3 text-sm">
          {(['read', 'all', 'custom'] as const).map((p) => (
            <label key={p} className="flex items-center gap-1">
              <input type="radio" checked={preset === p} onChange={() => setPreset(p)} />
              {p === 'read' ? 'Read-only' : p === 'all' ? 'Everything my role allows' : 'Choose'}
            </label>
          ))}
        </div>
        {preset === 'custom' && (
          <div className="grid max-h-40 grid-cols-2 gap-1 overflow-y-auto text-xs">
            {grantable.map((p) => (
              <label key={p} className="flex items-center gap-1">
                <input type="checkbox" checked={custom.includes(p)} onChange={(e) => setCustom(e.target.checked ? [...custom, p] : custom.filter((x) => x !== p))} />
                {p}
              </label>
            ))}
          </div>
        )}
        <Button size="sm" disabled={!name.trim() || !chosen.length} onClick={() => void create()}>
          Create token
        </Button>
      </div>
    </Card>
  );
}
