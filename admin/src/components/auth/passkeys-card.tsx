'use client';

/** My account → Passkeys: sign in with Face ID, Touch ID, Windows Hello or a security key. */
import React, { useState } from 'react';
import { Card, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { ErrorBanner } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { createPasskey, passkeysSupported } from '@/lib/passkey-client';
import { formatRelativeTime } from '@/lib/utils/format';
import type { PasskeyInfo } from '@/lib/db/repos/passkeys';

export function PasskeysCard() {
  const { data, reload } = useApi<{ passkeys: PasskeyInfo[] }>('/api/account/passkeys');
  const [name, setName] = useState('');
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const add = async () => {
    setErr(null);
    setBusy(true);
    try {
      const o = await api<{ challengeId: string; publicKey: Parameters<typeof createPasskey>[0] }>('/api/account/passkeys', { body: { action: 'options' } });
      const credential = await createPasskey(o.publicKey);
      await api('/api/account/passkeys', { body: { action: 'register', challengeId: o.challengeId, name: name || 'Passkey', credential } });
      setName('');
      await reload();
    } catch (e) {
      setErr(e instanceof DOMException && e.name === 'NotAllowedError' ? 'The passkey request was cancelled' : errorText(e));
    } finally {
      setBusy(false);
    }
  };
  const remove = async (p: PasskeyInfo) => {
    if (!confirm(`Remove "${p.name}"? You can no longer sign in with it.`)) return;
    try {
      await api(`/api/account/passkeys?id=${encodeURIComponent(p.credentialId)}`, { method: 'DELETE' });
      await reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };

  return (
    <Card>
      <CardHeader className="flex-col items-start gap-1">
        <CardTitle>Passkeys</CardTitle>
        <CardDescription>Sign in with your device&apos;s screen lock or a security key. A passkey with a PIN or biometric counts as two-factor sign-in.</CardDescription>
      </CardHeader>
      <ErrorBanner message={err} />
      <ul className="mb-3 space-y-1 text-sm">
        {data?.passkeys.length === 0 && <li className="text-foreground-tertiary">No passkeys yet.</li>}
        {data?.passkeys.map((p) => (
          <li key={p.credentialId} className="flex items-center justify-between gap-2">
            <span>
              {p.name} <span className="text-xs text-foreground-tertiary">added {formatRelativeTime(p.createdAt)}{p.lastUsedAt && ` · used ${formatRelativeTime(p.lastUsedAt)}`}</span>
            </span>
            <Button size="sm" variant="ghost" onClick={() => void remove(p)}>
              Remove
            </Button>
          </li>
        ))}
      </ul>
      {passkeysSupported() ? (
        <div className="flex flex-wrap items-end gap-2">
          <Input label="Name" placeholder="MacBook Touch ID" value={name} onChange={(e) => setName(e.target.value)} />
          <Button loading={busy} onClick={() => void add()}>
            Add a passkey
          </Button>
        </div>
      ) : (
        <p className="text-xs text-foreground-tertiary">This browser does not support passkeys.</p>
      )}
    </Card>
  );
}
