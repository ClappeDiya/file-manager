'use client';

/** Account → Password: change your own password (accounts with a password). */
import React, { useState } from 'react';
import { Card, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { ErrorBanner } from '@/components/fleet/badges';
import { PasswordStrength } from './password-strength';
import { api, errorText } from '@/lib/api-client';
import { checkPassword } from '@/lib/password-policy';
import { useToastStore } from '@/lib/stores/toast-store';

export function ChangePassword({ email, name }: { email: string; name: string }) {
  const notify = useToastStore((s) => s.notify);
  const [current, setCurrent] = useState('');
  const [password, setPassword] = useState('');
  const [confirm, setConfirm] = useState('');
  const [err, setErr] = useState<string | null>(null);
  const ok = !!current && checkPassword(password, { email, name }).ok && password === confirm;

  const save = async (e: React.FormEvent) => {
    e.preventDefault();
    setErr(null);
    try {
      await api('/api/account/password', { body: { current, password } });
      setCurrent('');
      setPassword('');
      setConfirm('');
      notify('Password changed', 'success');
    } catch (x) {
      setErr(errorText(x));
    }
  };

  return (
    <Card>
      <CardHeader className="flex-col items-start gap-1">
        <CardTitle>Password</CardTitle>
        <CardDescription>At least 12 characters; not your name or address. Your organization may require a change after a set number of days.</CardDescription>
      </CardHeader>
      <form onSubmit={save} className="space-y-3">
        <ErrorBanner message={err} />
        <Input label="Current password" type="password" autoComplete="current-password" value={current} onChange={(e) => setCurrent(e.target.value)} />
        <Input label="New password" type="password" autoComplete="new-password" value={password} onChange={(e) => setPassword(e.target.value)} />
        <PasswordStrength password={password} email={email} name={name} />
        <Input label="Repeat the new password" type="password" autoComplete="new-password" value={confirm} onChange={(e) => setConfirm(e.target.value)} />
        <Button type="submit" disabled={!ok}>
          Change password
        </Button>
      </form>
    </Card>
  );
}
