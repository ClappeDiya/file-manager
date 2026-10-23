'use client';

/**
 * Password reset: ask for a link by e-mail, or (with ?token=) choose a new
 * password — also used when a password has expired at sign-in.
 */
import React, { Suspense, useEffect, useState } from 'react';
import Link from 'next/link';
import { useRouter, useSearchParams } from 'next/navigation';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { Button } from '@/components/ui/button';
import { PasswordStrength } from '@/components/auth/password-strength';
import { checkPassword } from '@/lib/password-policy';
import { useAuthStore } from '@/lib/stores/auth-store';

const safeRedirect = (v: string | null) => (v && v.startsWith('/') && !v.startsWith('//') ? v : '/dashboard');

function ResetForm() {
  const params = useSearchParams();
  const router = useRouter();
  const token = params.get('token');
  const redirect = safeRedirect(params.get('redirect'));
  const validateSession = useAuthStore((s) => s.validateSession);
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [confirm, setConfirm] = useState('');
  const [info, setInfo] = useState<{ email: string; purpose: string } | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!token) return;
    fetch(`/api/auth/password/reset?token=${encodeURIComponent(token)}`)
      .then(async (r) => (r.ok ? setInfo(await r.json()) : setError((await r.json()).error)))
      .catch(() => setError('Could not check the link'));
  }, [token]);

  const post = async (url: string, body: unknown) => {
    setBusy(true);
    setError(null);
    try {
      const r = await fetch(url, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });
      const data = await r.json();
      if (!r.ok) throw new Error(data.error ?? 'Request failed');
      return data;
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      return null;
    } finally {
      setBusy(false);
    }
  };

  const request = async (e: React.FormEvent) => {
    e.preventDefault();
    const d = await post('/api/auth/password/forgot', { email });
    if (d) setMessage(d.message);
  };

  const reset = async (e: React.FormEvent) => {
    e.preventDefault();
    if (password !== confirm) return setError('The passwords do not match');
    const d = await post('/api/auth/password/reset', { token, password });
    if (!d) return;
    if (d.mfaRequired) router.replace(`/login?mfa=${encodeURIComponent(d.challengeId)}&redirect=${encodeURIComponent(redirect)}`);
    else {
      await validateSession();
      router.replace(redirect);
    }
  };

  const ok = checkPassword(password, { email: info?.email }).ok && password === confirm;
  return (
    <div className="flex min-h-screen items-center justify-center bg-background-secondary p-4">
      <Card className="w-full max-w-sm">
        <CardHeader className="flex-col items-start gap-1">
          <CardTitle>{token ? (info?.purpose === 'expired' ? 'Your password has expired' : 'Choose a new password') : 'Reset your password'}</CardTitle>
          <CardDescription>{token ? info?.email ?? '' : 'We e-mail you a link that is valid for one hour.'}</CardDescription>
        </CardHeader>
        <CardContent>
          {error && (
            <p role="alert" className="mb-3 text-sm text-error">
              {error}
            </p>
          )}
          {!token ? (
            message ? (
              <p className="text-sm">{message}</p>
            ) : (
              <form onSubmit={request} className="space-y-3">
                <Input label="Email" type="email" value={email} onChange={(e) => setEmail(e.target.value)} autoComplete="email" required />
                <Button type="submit" className="w-full" loading={busy}>
                  Send reset link
                </Button>
              </form>
            )
          ) : info ? (
            <form onSubmit={reset} className="space-y-3">
              <Input label="New password" type="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="new-password" required />
              <PasswordStrength password={password} email={info.email} />
              <Input label="Repeat the password" type="password" value={confirm} onChange={(e) => setConfirm(e.target.value)} autoComplete="new-password" required />
              <Button type="submit" className="w-full" loading={busy} disabled={!ok}>
                Save and sign in
              </Button>
            </form>
          ) : null}
          <Link href="/login" className="mt-3 block text-center text-xs text-primary hover:underline">
            Back to sign in
          </Link>
        </CardContent>
      </Card>
    </div>
  );
}

export default function ResetPasswordPage() {
  return (
    <Suspense fallback={null}>
      <ResetForm />
    </Suspense>
  );
}
