'use client';

/** My account → Sessions: where you are signed in, and signing out elsewhere. */
import React, { useState } from 'react';
import { Card, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { ErrorBanner } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { formatRelativeTime } from '@/lib/utils/format';
import type { Session } from '@/lib/db/repos/sessions';

const browser = (ua: string) => (/(Edg|Chrome|Firefox|Safari)\/[\d.]+/.exec(ua)?.[0]?.replace('/', ' ') ?? (ua || 'Unknown client'));

export function SessionsCard() {
  const { data, reload } = useApi<{ sessions: (Session & { current: boolean })[] }>('/api/account/sessions', 60_000);
  const [err, setErr] = useState<string | null>(null);
  const end = async (q: string) => {
    setErr(null);
    try {
      await api(`/api/account/sessions?${q}`, { method: 'DELETE' });
      await reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };
  const others = (data?.sessions ?? []).filter((s) => !s.current);
  return (
    <Card>
      <CardHeader className="flex-col items-start gap-1">
        <CardTitle>Sessions</CardTitle>
        <CardDescription>Where your account is signed in. Changing your password signs out the others.</CardDescription>
      </CardHeader>
      <ErrorBanner message={err} />
      <ul className="mb-3 space-y-1 text-sm">
        {data?.sessions.map((s) => (
          <li key={s.id} className="flex items-center justify-between gap-2">
            <span>
              {browser(s.userAgent)} · {s.ip || 'unknown address'} <span className="text-xs text-foreground-tertiary">· active {formatRelativeTime(s.lastSeenAt)} · {s.method}</span>
              {s.current && <span className="ml-1 text-xs font-medium text-success">this session</span>}
            </span>
            {!s.current && (
              <Button size="sm" variant="ghost" onClick={() => void end(`id=${encodeURIComponent(s.id)}`)}>
                Sign out
              </Button>
            )}
          </li>
        ))}
      </ul>
      {others.length > 0 && (
        <Button size="sm" variant="outline" onClick={() => void end('others=1')}>
          Sign out all other sessions
        </Button>
      )}
    </Card>
  );
}
