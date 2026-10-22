'use client';

/** Organization security settings: password age (and what sign-in enforces). */
import React, { useState } from 'react';
import { PageHeader } from '@/components/layout/page-header';
import { Card, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { ErrorBanner, Loading } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { useAuthStore } from '@/lib/stores/auth-store';
import { useToastStore } from '@/lib/stores/toast-store';
import type { SecuritySettings } from '@/lib/db/repos/security';

export default function SecuritySettingsPage() {
  const { data, error, loading, reload } = useApi<{ settings: SecuritySettings }>('/api/security');
  const canWrite = useAuthStore((s) => s.checkPermission)('sso.configure');
  const notify = useToastStore((s) => s.notify);
  const [draft, setDraft] = useState<SecuritySettings | null>(null);
  const [err, setErr] = useState<string | null>(null);
  if (loading && !data) return <Loading />;
  const s = draft ?? data?.settings;
  if (!s) return <ErrorBanner message={error ?? 'Could not load the settings'} />;

  const save = async () => {
    setErr(null);
    try {
      await api('/api/security', { method: 'PUT', body: s });
      setDraft(null);
      notify('Security settings saved', 'success');
      await reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };

  return (
    <div>
      <PageHeader title="Security" description="Rules for every console account in this organization." />
      <ErrorBanner message={err} />
      <fieldset disabled={!canWrite} className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader className="flex-col items-start gap-1">
            <CardTitle>Passwords</CardTitle>
            <CardDescription>
              Passwords need 12+ characters, no name or address, and are checked against common passwords. Accounts that belong to several organizations follow the strictest age.
            </CardDescription>
          </CardHeader>
          <Input
            label="Require a new password after (days, 0 = never)"
            type="number"
            min={0}
            max={3650}
            value={s.passwordMaxAgeDays}
            onChange={(e) => setDraft({ ...s, passwordMaxAgeDays: Math.max(0, Math.floor(Number(e.target.value) || 0)) })}
          />
        </Card>
        <Card>
          <CardHeader className="flex-col items-start gap-1">
            <CardTitle>Two-factor sign-in</CardTitle>
            <CardDescription>Members without an authenticator app can only set one up until they do. Members of several organizations follow the strictest rule.</CardDescription>
          </CardHeader>
          <div className="space-y-2 text-sm">
            <label className="flex items-center gap-2">
              <input type="checkbox" checked={s.requireMfa} onChange={(e) => setDraft({ ...s, requireMfa: e.target.checked })} />
              Require two-factor sign-in for every member
            </label>
            <label className="flex items-center gap-2">
              <input type="checkbox" disabled={!s.requireMfa} checked={s.ssoExemptFromMfa} onChange={(e) => setDraft({ ...s, ssoExemptFromMfa: e.target.checked })} />
              Except single sign-on (your identity provider enforces MFA)
            </label>
          </div>
        </Card>
      </fieldset>
      {canWrite && (
        <Button className="mt-4" disabled={!draft} onClick={() => void save()}>
          Save
        </Button>
      )}
    </div>
  );
}
