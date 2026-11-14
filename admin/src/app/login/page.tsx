'use client';

import Link from 'next/link';
import { getPasskey, passkeysSupported } from '@/lib/passkey-client';
import React, { Suspense, useEffect, useState } from 'react';
import { useRouter, useSearchParams } from 'next/navigation';
import { Fingerprint, KeyRound } from 'lucide-react';
import { useAuthStore } from '@/lib/stores/auth-store';
import { useTranslate } from '@/lib/i18n';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Card, CardHeader, CardTitle, CardDescription, CardContent, CardFooter } from '@/components/ui/card';

interface SsoOption {
  id: string;
  name: string;
  kind: 'oidc' | 'saml' | 'ldap';
}

const safeRedirect = (v: string | null) => (v && v.startsWith('/') && !v.startsWith('//') ? v : '/dashboard');

function LoginForm() {
  const router = useRouter();
  const params = useSearchParams();
  const { isAuthenticated, isLoading, error, setError, login, verifyMfa, validateSession } = useAuthStore();
  const redirect = safeRedirect(params.get('redirect'));
  const t = useTranslate();
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [code, setCode] = useState('');
  const [challengeId, setChallengeId] = useState<string | null>(params.get('mfa'));
  const [sso, setSso] = useState<SsoOption[]>([]);
  /** An LDAP provider chosen: sign in with directory credentials instead. */
  const [directory, setDirectory] = useState<SsoOption | null>(null);
  const [username, setUsername] = useState('');
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const e = params.get('error');
    if (e) setError(e);
  }, [params, setError]);

  useEffect(() => {
    if (isAuthenticated) router.replace(redirect);
  }, [isAuthenticated, router, redirect]);

  // Offer the organization's sign-in methods once the address names a domain.
  useEffect(() => {
    const domain = email.split('@')[1] ?? '';
    if (!/\.[a-z]{2,}$/i.test(domain)) {
      setSso([]);
      return;
    }
    const ctl = new AbortController();
    const t = setTimeout(() => {
      fetch(`/api/auth/sso/discover?email=${encodeURIComponent(email)}`, { signal: ctl.signal })
        .then((r) => r.json())
        .then((d: { providers?: SsoOption[] }) => setSso(d.providers ?? []))
        .catch(() => {});
    }, 300);
    return () => (clearTimeout(t), ctl.abort());
  }, [email]);

  const submitPassword = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);
    if (!email || !password) return setError('Please enter both email and password');
    const r = await login(email, password, redirect);
    if (r.ok) router.replace(safeRedirect(r.redirect));
    else if ('mfaChallengeId' in r) setChallengeId(r.mfaChallengeId);
    else if ('passwordChangeToken' in r) router.replace(`/reset-password?token=${encodeURIComponent(r.passwordChangeToken)}&redirect=${encodeURIComponent(redirect)}`);
  };

  const submitDirectory = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!directory) return;
    setError(null);
    setBusy(true);
    try {
      const r = await fetch('/api/auth/ldap', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ providerId: directory.id, username, password, redirect }) });
      const data = await r.json();
      if (!r.ok) throw new Error(data.error ?? 'Sign-in failed');
      setPassword('');
      if (data.mfaRequired) return setChallengeId(data.challengeId);
      await validateSession();
      router.replace(safeRedirect(data.redirect ?? redirect));
    } catch (x) {
      setError(x instanceof Error ? x.message : String(x));
    } finally {
      setBusy(false);
    }
  };

  const signInWithPasskey = async () => {
    setError(null);
    try {
      const o = await (await fetch('/api/auth/passkey/options', { method: 'POST' })).json();
      const credential = await getPasskey(o.publicKey);
      const r = await fetch('/api/auth/passkey/verify', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ challengeId: o.challengeId, credential }) });
      const data = await r.json();
      if (!r.ok) throw new Error(data.error ?? 'Passkey sign-in failed');
      if (data.mfaRequired) return setChallengeId(data.challengeId);
      await validateSession();
      router.replace(safeRedirect(data.mfaSetupRequired ? data.redirect : redirect));
    } catch (x) {
      setError(x instanceof DOMException && x.name === 'NotAllowedError' ? 'The passkey request was cancelled' : x instanceof Error ? x.message : String(x));
    }
  };

  const submitCode = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!challengeId) return;
    const r = await verifyMfa(challengeId, code.trim(), redirect);
    if (r.ok) router.replace(safeRedirect(r.redirect));
  };

  const errorBox = error && (
    <div role="alert" className="rounded-md bg-error-bg border border-error/20 px-3 py-2">
      <p className="text-sm text-error">{error}</p>
    </div>
  );

  return (
    <div className="min-h-screen flex items-center justify-center bg-background-secondary p-4">
      <div className="w-full max-w-sm">
        <div className="flex flex-col items-center mb-8">
          <div className="w-12 h-12 rounded-xl bg-primary flex items-center justify-center mb-3">
            <span className="text-primary-foreground font-bold text-xl">U</span>
          </div>
          <h1 className="text-xl font-bold text-foreground">{t('app.name')}</h1>
          <p className="text-sm text-foreground-secondary mt-1">{t('app.tagline')}</p>
        </div>

        {challengeId ? (
          <Card>
            <CardHeader className="flex-col items-start gap-1">
              <CardTitle>{t('login.mfaTitle')}</CardTitle>
              <CardDescription>{t('login.mfaDescription')}</CardDescription>
            </CardHeader>
            <CardContent>
              <form onSubmit={submitCode} className="space-y-4">
                <Input label={t('login.code')} inputMode="numeric" autoComplete="one-time-code" value={code} onChange={(e) => setCode(e.target.value)} autoFocus required />
                {errorBox}
                <Button type="submit" className="w-full" loading={isLoading}>
                  {t('login.verify')}
                </Button>
                <Button type="button" variant="ghost" className="w-full" onClick={() => (setChallengeId(null), setCode(''), setError(null))}>
                  {t('login.back')}
                </Button>
              </form>
            </CardContent>
          </Card>
        ) : directory ? (
          <Card>
            <CardHeader className="flex-col items-start gap-1">
              <CardTitle>{t('login.directorySignIn', { name: directory.name })}</CardTitle>
            </CardHeader>
            <CardContent>
              <form onSubmit={submitDirectory} className="space-y-4">
                <Input label={t('login.directoryUser')} value={username} onChange={(e) => setUsername(e.target.value)} autoComplete="username" autoFocus required />
                <Input label={t('login.password')} type="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="current-password" required />
                {errorBox}
                <Button type="submit" className="w-full" loading={busy}>
                  {t('login.submit')}
                </Button>
                <Button type="button" variant="ghost" className="w-full" onClick={() => (setDirectory(null), setPassword(''), setError(null))}>
                  {t('login.back')}
                </Button>
              </form>
            </CardContent>
          </Card>
        ) : (
          <Card>
            <CardHeader className="flex-col items-start gap-1">
              <CardTitle>{t('login.title')}</CardTitle>
              <CardDescription>{t('login.description')}</CardDescription>
            </CardHeader>
            <CardContent>
              <form onSubmit={submitPassword} className="space-y-4">
                <Input label={t('login.email')} type="email" placeholder="you@company.com" value={email} onChange={(e) => setEmail(e.target.value)} autoComplete="email" required />
                <Input label={t('login.password')} type="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="current-password" />
                {errorBox}
                <Button type="submit" className="w-full" loading={isLoading}>
                  {t('login.submit')}
                </Button>
                {passkeysSupported() && (
                  <Button type="button" variant="outline" className="w-full" onClick={() => void signInWithPasskey()}>
                    <Fingerprint size={14} className="mr-2" aria-hidden="true" />
                    {t('login.passkey')}
                  </Button>
                )}
                <Link href="/reset-password" className="block text-center text-xs text-primary hover:underline">
                  {t('login.forgot')}
                </Link>
              </form>
            </CardContent>
            {sso.length > 0 && (
              <CardFooter className="flex-col gap-2">
                <p className="w-full text-xs text-foreground-tertiary">{t('login.orContinue')}</p>
                {sso.map((p) => (
                  <Button
                    key={p.id}
                    variant="outline"
                    className="w-full"
                    onClick={() => {
                      if (p.kind === 'ldap') {
                        setDirectory(p);
                        setUsername(email);
                        setError(null);
                      } else window.location.href = `/api/auth/sso/${p.id}/start?redirect=${encodeURIComponent(redirect)}`;
                    }}
                  >
                    <KeyRound size={14} className="mr-2" aria-hidden="true" />
                    {p.name} ({p.kind.toUpperCase()})
                  </Button>
                ))}
              </CardFooter>
            )}
          </Card>
        )}
      </div>
    </div>
  );
}

export default function LoginPage() {
  return (
    <Suspense fallback={null}>
      <LoginForm />
    </Suspense>
  );
}
