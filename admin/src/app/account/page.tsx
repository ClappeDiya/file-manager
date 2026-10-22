'use client';

import React, { Suspense, useEffect, useState } from 'react';
import { useSearchParams } from 'next/navigation';
import QRCode from 'qrcode';
import { KeyRound, ShieldCheck } from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { ErrorBanner, Loading } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { useAuthStore } from '@/lib/stores/auth-store';
import { useToastStore } from '@/lib/stores/toast-store';
import { ROLE_LABELS } from '@/lib/types/auth';
import { formatRelativeTime } from '@/lib/utils/format';

interface Identities {
  hasPassword: boolean;
  identities: { id: string; providerName: string; kind: string; email: string; createdAt: string; lastUsedAt: string | null }[];
  available: { id: string; name: string; kind: string }[];
}

function AccountView() {
  const params = useSearchParams();
  const user = useAuthStore((s) => s.user);
  const organization = useAuthStore((s) => s.organization);
  const notify = useToastStore((s) => s.notify);
  const mfa = useApi<{ enabled: boolean; pending: boolean; recoveryCodesLeft: number }>('/api/account/mfa');
  const ids = useApi<Identities>('/api/account/identities');
  const [setup, setSetup] = useState<{ secret: string; otpauthUrl: string; qr: string } | null>(null);
  const [code, setCode] = useState('');
  const [codes, setCodes] = useState<string[] | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    const linked = params.get('linked');
    if (linked) notify(`${linked} is now linked to your account`, 'success');
  }, [params, notify]);

  const begin = async () => {
    setErr(null);
    try {
      const r = await api<{ secret: string; otpauthUrl: string }>('/api/account/mfa', { body: { action: 'begin' } });
      setSetup({ ...r, qr: await QRCode.toDataURL(r.otpauthUrl, { margin: 1, width: 192 }) });
    } catch (e) {
      setErr(errorText(e));
    }
  };

  const confirmCode = async () => {
    setErr(null);
    try {
      const r = await api<{ recoveryCodes: string[] }>('/api/account/mfa', { body: { action: 'confirm', code: code.trim() } });
      setCodes(r.recoveryCodes);
      setSetup(null);
      setCode('');
      await mfa.reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };

  const disable = async () => {
    setErr(null);
    try {
      await api('/api/account/mfa', { body: { action: 'disable', code: code.trim() } });
      setCode('');
      await mfa.reload();
      notify('Two-factor sign-in is off', 'info');
    } catch (e) {
      setErr(errorText(e));
    }
  };

  const unlink = async (id: string) => {
    setErr(null);
    try {
      await api(`/api/account/identities?id=${id}`, { method: 'DELETE' });
      await ids.reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };

  return (
    <div>
      <PageHeader title="My account" description={user ? `${user.name} · ${user.email}${organization ? ` · ${ROLE_LABELS[user.role]} in ${organization.name}` : ''}` : ''} />
      <ErrorBanner message={err ?? mfa.error ?? ids.error} />
      <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
        <Card>
          <CardHeader className="flex-col items-start gap-1">
            <CardTitle>
              <ShieldCheck size={16} className="mr-2 inline" aria-hidden="true" />
              Two-factor sign-in
            </CardTitle>
            <CardDescription>Ask for a code from an authenticator app (TOTP) after your password or single sign-on.</CardDescription>
          </CardHeader>
          {!mfa.data ? (
            <Loading />
          ) : codes ? (
            <div className="space-y-3">
              <p className="text-sm text-foreground">Two-factor sign-in is on. Save these recovery codes somewhere safe. Each works once, and they are not shown again.</p>
              <pre className="grid grid-cols-2 gap-1 rounded bg-background-secondary p-3 font-mono text-sm">{codes.map((c) => <span key={c}>{c}</span>)}</pre>
              <div className="flex gap-2">
                <Button variant="outline" onClick={() => void navigator.clipboard.writeText(codes.join('\n'))}>
                  Copy
                </Button>
                <Button onClick={() => setCodes(null)}>I saved them</Button>
              </div>
            </div>
          ) : mfa.data.enabled ? (
            <div className="space-y-3">
              <p className="text-sm text-success">On · {mfa.data.recoveryCodesLeft} recovery code(s) left</p>
              <div className="flex flex-wrap items-end gap-2">
                <Input label="Current code (to turn it off)" inputMode="numeric" autoComplete="one-time-code" value={code} onChange={(e) => setCode(e.target.value)} />
                <Button variant="danger" onClick={() => void disable()} disabled={code.trim().length < 6}>
                  Turn off
                </Button>
              </div>
            </div>
          ) : setup ? (
            <div className="space-y-3">
              <p className="text-sm">Scan the code with your authenticator app, or enter the key by hand.</p>
              <img src={setup.qr} alt="QR code for your authenticator app" width={192} height={192} className="rounded border border-border bg-white" />
              <code className="block break-all rounded bg-background-secondary p-2 text-xs">{setup.secret}</code>
              <div className="flex flex-wrap items-end gap-2">
                <Input label="Code from the app" inputMode="numeric" autoComplete="one-time-code" value={code} onChange={(e) => setCode(e.target.value)} autoFocus />
                <Button onClick={() => void confirmCode()} disabled={code.trim().length < 6}>
                  Confirm
                </Button>
              </div>
            </div>
          ) : (
            <Button onClick={() => void begin()}>Set up two-factor sign-in</Button>
          )}
        </Card>

        <Card>
          <CardHeader className="flex-col items-start gap-1">
            <CardTitle>
              <KeyRound size={16} className="mr-2 inline" aria-hidden="true" />
              Sign-in methods
            </CardTitle>
            <CardDescription>Link your organization&apos;s identity providers to sign in without a password.</CardDescription>
          </CardHeader>
          {!ids.data ? (
            <Loading />
          ) : (
            <div className="space-y-3">
              <ul className="divide-y divide-border text-sm">
                <li className="flex justify-between py-2">
                  <span>Password</span>
                  <span className="text-foreground-secondary">{ids.data.hasPassword ? 'set' : 'not set'}</span>
                </li>
                {ids.data.identities.map((i) => (
                  <li key={i.id} className="flex items-center justify-between py-2">
                    <span>
                      {i.providerName} <span className="text-xs text-foreground-tertiary">{i.kind.toUpperCase()} · {i.email} · {i.lastUsedAt ? `used ${formatRelativeTime(i.lastUsedAt)}` : 'never used'}</span>
                    </span>
                    <Button size="sm" variant="ghost" onClick={() => void unlink(i.id)}>
                      Unlink
                    </Button>
                  </li>
                ))}
              </ul>
              {ids.data.available.length > 0 && (
                <div className="flex flex-wrap gap-2">
                  {ids.data.available.map((p) => (
                    <Button key={p.id} variant="outline" onClick={() => (window.location.href = `/api/auth/sso/${p.id}/start?link=1`)}>
                      Link {p.name}
                    </Button>
                  ))}
                </div>
              )}
            </div>
          )}
        </Card>
      </div>
    </div>
  );
}

export default function AccountPage() {
  return (
    <Suspense fallback={<Loading />}>
      <AccountView />
    </Suspense>
  );
}
