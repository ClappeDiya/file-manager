'use client';

/**
 * Browser sign-in from the desktop app: the app opens this page with a
 * loopback `redirect_uri`, `state` and PKCE `code_challenge`. After signing
 * in (password, SSO, MFA), the user approves and the browser returns to the
 * app with a one-time code that only that app can redeem.
 */
import React, { Suspense, useState } from 'react';
import { useSearchParams } from 'next/navigation';
import { Monitor } from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { ErrorBanner } from '@/components/fleet/badges';
import { api, errorText } from '@/lib/api-client';
import { useAuthStore } from '@/lib/stores/auth-store';

const LOOPBACK = /^http:\/\/(127\.0\.0\.1|\[::1\]|localhost):\d+\//;

function ConnectForm() {
  const params = useSearchParams();
  const org = useAuthStore((s) => s.organization);
  const canEnroll = useAuthStore((s) => s.checkPermission)('fleet.enroll');
  const redirectUri = params.get('redirect_uri') ?? '';
  const state = params.get('state') ?? '';
  const codeChallenge = params.get('code_challenge') ?? '';
  const host = params.get('hostname') ?? '';
  const [name, setName] = useState(params.get('name') ?? host);
  const [tags, setTags] = useState('');
  const [err, setErr] = useState<string | null>(null);
  const [done, setDone] = useState(false);

  const valid = LOOPBACK.test(redirectUri) && !!state && !!codeChallenge && params.get('code_challenge_method') === 'S256';
  const back = (extra: Record<string, string>) => {
    const u = new URL(redirectUri);
    u.searchParams.set('state', state);
    for (const [k, v] of Object.entries(extra)) u.searchParams.set(k, v);
    window.location.href = u.toString();
  };

  const approve = async () => {
    setErr(null);
    try {
      const r = await api<{ redirect: string }>('/api/fleet/connect', { body: { redirectUri, state, codeChallenge, name: name.trim(), tags: tags.split(',').map((t) => t.trim()).filter(Boolean) } });
      setDone(true);
      window.location.href = r.redirect;
    } catch (e) {
      setErr(errorText(e));
    }
  };

  return (
    <div>
      <PageHeader title="Connect a desktop" description="Register the UFOP app that opened this page with your organization." />
      <Card className="max-w-xl">
        <CardHeader>
          <CardTitle>
            <Monitor size={16} className="mr-2 inline" aria-hidden="true" />
            {host || 'This computer'}
          </CardTitle>
        </CardHeader>
        {!valid ? (
          <ErrorBanner message="This sign-in link is incomplete. Start again from the app (Settings → Managed by Console → Sign in with browser)." />
        ) : done ? (
          <CardDescription>Returning to the app… You can close this tab once the app shows it is connected.</CardDescription>
        ) : !canEnroll ? (
          <>
            <ErrorBanner message="Your role cannot enroll machines. Ask an administrator for a registration link." />
            <Button variant="outline" className="mt-3" onClick={() => back({ error: 'access_denied' })}>
              Back to the app
            </Button>
          </>
        ) : (
          <div className="space-y-3">
            <CardDescription>
              The machine joins <strong>{org?.name ?? 'your organization'}</strong> and receives its policies. Only approve if you started this from the UFOP app on {host || 'your computer'} just now.
            </CardDescription>
            <Input label="Machine name" value={name} onChange={(e) => setName(e.target.value)} />
            <Input label="Tags (comma-separated)" value={tags} onChange={(e) => setTags(e.target.value)} placeholder="laptop, finance" />
            {err && <ErrorBanner message={err} />}
            <div className="flex gap-2">
              <Button onClick={() => void approve()}>Approve and connect</Button>
              <Button variant="outline" onClick={() => back({ error: 'access_denied' })}>
                Deny
              </Button>
            </div>
          </div>
        )}
      </Card>
    </div>
  );
}

export default function ConnectPage() {
  return (
    <Suspense fallback={null}>
      <ConnectForm />
    </Suspense>
  );
}
