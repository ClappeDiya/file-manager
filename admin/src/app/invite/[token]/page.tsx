'use client';

import React, { use, useEffect, useState } from 'react';
import Link from 'next/link';
import { useRouter } from 'next/navigation';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { useAuthStore } from '@/lib/stores/auth-store';
import { ROLE_LABELS, type Role } from '@/lib/types/auth';
import { formatDateTime } from '@/lib/utils/format';

interface InviteInfo {
  email: string;
  role: Role;
  organization: string;
  status: string;
  expiresAt: string;
}

export default function InvitePage({ params }: { params: Promise<{ token: string }> }) {
  const { token: raw } = use(params);
  const token = decodeURIComponent(raw);
  const router = useRouter();
  const { user, validateSession } = useAuthStore();
  // The shell validates the session on load; this page only refreshes it after accepting.
  const [info, setInfo] = useState<InviteInfo | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState('');
  const [password, setPassword] = useState('');
  const [confirm, setConfirm] = useState('');
  const [busy, setBusy] = useState(false);
  const [mfaNeeded, setMfaNeeded] = useState(false);

  useEffect(() => {
    fetch(`/api/invitations/accept?token=${encodeURIComponent(token)}`)
      .then(async (r) => {
        const d = await r.json();
        if (!r.ok) throw new Error(d.error || 'This invitation link is not valid');
        setInfo(d);
      })
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, [token]);

  const accept = async (e?: React.FormEvent) => {
    e?.preventDefault();
    setError(null);
    if (!user && password !== confirm) return setError('The passwords do not match');
    setBusy(true);
    try {
      const r = await fetch('/api/invitations/accept', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(user ? { token } : { token, name, password }) });
      const d = await r.json();
      if (!r.ok) throw new Error(d.error || 'Could not accept the invitation');
      if (d.mfaRequired) {
        setMfaNeeded(true);
        router.replace(`/login?mfa=${encodeURIComponent(d.challengeId)}`);
        return;
      }
      await validateSession();
      router.replace('/dashboard');
    } catch (e2) {
      setError(e2 instanceof Error ? e2.message : String(e2));
    } finally {
      setBusy(false);
    }
  };

  const usable = info?.status === 'pending';
  return (
    <div className="min-h-screen flex items-center justify-center bg-background-secondary p-4">
      <div className="w-full max-w-sm">
        <Card>
          <CardHeader className="flex-col items-start gap-1">
            <CardTitle>Join {info?.organization || 'the organization'}</CardTitle>
            {info && (
              <CardDescription>
                {info.email} is invited as {ROLE_LABELS[info.role]}. {usable ? `The invitation expires ${formatDateTime(info.expiresAt)}.` : `This invitation is ${info.status}.`}
              </CardDescription>
            )}
          </CardHeader>
          <CardContent>
            {error && (
              <div role="alert" className="mb-3 rounded-md bg-error-bg border border-error/20 px-3 py-2">
                <p className="text-sm text-error">{error}</p>
              </div>
            )}
            {mfaNeeded ? null : !info ? null : !usable ? (
              <Link href="/login" className="text-sm text-primary">
                Go to sign in
              </Link>
            ) : user ? (
              <div className="space-y-3">
                <p className="text-sm text-foreground-secondary">You are signed in as {user.email}.</p>
                <Button className="w-full" loading={busy} onClick={() => void accept()}>
                  Accept invitation
                </Button>
              </div>
            ) : (
              <form onSubmit={accept} className="space-y-3">
                <Input label="Your name" value={name} onChange={(e) => setName(e.target.value)} autoComplete="name" required />
                <Input label="Choose a password" type="password" value={password} onChange={(e) => setPassword(e.target.value)} autoComplete="new-password" hint="At least 12 characters" required />
                <Input label="Repeat the password" type="password" value={confirm} onChange={(e) => setConfirm(e.target.value)} autoComplete="new-password" required />
                <Button type="submit" className="w-full" loading={busy}>
                  Create account and join
                </Button>
                <p className="text-xs text-foreground-tertiary">
                  Already have an account?{' '}
                  <Link className="text-primary" href={`/login?redirect=${encodeURIComponent(`/invite/${encodeURIComponent(token)}`)}`}>
                    Sign in first
                  </Link>
                  .
                </p>
              </form>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
