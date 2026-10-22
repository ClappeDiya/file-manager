'use client';

/** My account → Profile: display name and the time zone the console shows times in. */
import React, { useMemo, useState } from 'react';
import { Card, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { ErrorBanner } from '@/components/fleet/badges';
import { api, errorText } from '@/lib/api-client';
import { useAuthStore } from '@/lib/stores/auth-store';
import { useToastStore } from '@/lib/stores/toast-store';
import { formatDateTime } from '@/lib/utils/format';
import type { AdminUser } from '@/lib/types/auth';

const zones = (): string[] => {
  try {
    return (Intl as unknown as { supportedValuesOf(k: string): string[] }).supportedValuesOf('timeZone');
  } catch {
    return ['UTC'];
  }
};

export function ProfileCard() {
  const user = useAuthStore((s) => s.user);
  const setUser = useAuthStore((s) => s.setUser);
  const notify = useToastStore((s) => s.notify);
  const [name, setName] = useState<string | null>(null);
  const [tz, setTz] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const options = useMemo(
    () => [{ value: '', label: `Browser (${Intl.DateTimeFormat().resolvedOptions().timeZone})` }, ...['UTC', ...zones().filter((z) => z !== 'UTC')].map((z) => ({ value: z, label: z.replace(/_/g, ' ') }))],
    [],
  );
  if (!user) return null;
  const curTz = tz ?? user.timeZone ?? '';
  const save = async () => {
    setErr(null);
    try {
      const r = await api<{ user: AdminUser }>('/api/account/profile', { method: 'PATCH', body: { ...(name !== null ? { name } : {}), ...(tz !== null ? { timeZone: tz || null } : {}) } });
      setUser({ ...user, ...r.user, role: user.role });
      setName(null);
      setTz(null);
      notify('Profile saved', 'success');
    } catch (e) {
      setErr(errorText(e));
    }
  };
  return (
    <Card>
      <CardHeader className="flex-col items-start gap-1">
        <CardTitle>Profile</CardTitle>
        <CardDescription>Times in the console are shown in your time zone (now: {formatDateTime(new Date())}).</CardDescription>
      </CardHeader>
      <ErrorBanner message={err} />
      <div className="space-y-2">
        <Input label="Name" value={name ?? user.name} onChange={(e) => setName(e.target.value)} />
        <Select label="Time zone" options={options} value={curTz} onChange={(e) => setTz(e.target.value)} />
        <Button size="sm" disabled={name === null && tz === null} onClick={() => void save()}>
          Save
        </Button>
      </div>
    </Card>
  );
}
